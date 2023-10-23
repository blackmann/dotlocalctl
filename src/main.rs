use std::{
    collections::{HashMap, HashSet},
    env::args,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    process::{Child, Command, Stdio},
    thread::sleep,
};

use chrono::Local;
use clap::{Parser, Subcommand};
use core::time;
use local_ip_address::local_ip;
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Response, Server};

const ADDR: &str = "127.0.0.1:2023";

#[derive(Parser)]
#[command(
    author = "De-Great Yartey <mail@degreat.co.uk>",
    version = "0.1.0",
    about = "Run .local DNS resolution for apps in development",
    long_about = "Run .local DNS resolution for apps in development"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Setup dnslocalctl and related tools to be able to serve requests
    Configure,

    /// Run dnslocal server and blocks
    Run,

    /// Start dnslocal server in the background
    Start,

    /// Restarts server
    Restart,

    /// Stop server.
    Stop,

    /// Add a proxy entry in the format `<domain>:<port>`. You can add
    /// multiple records separated by space.
    ///
    /// Eg. `dnslocalctl add adeton.local:3000 mangobase.local:3003`
    Add {
        #[arg()]
        proxies: Vec<String>,
    },

    /// Remove a proxy entry or multiple entries.
    ///
    /// Eg. `dnslocalctl remove adeton.local:3000 mangobase.local:3003`
    Remove {
        #[arg()]
        proxies: Vec<String>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Record {
    domain: String,
    paths: Vec<(String, i32)>,
    port: i32,
}

impl Record {
    fn entry(&self, automatic_https_redirect: bool) -> String {
        let mut res = String::new();

        let domain = &self.domain;
        if automatic_https_redirect {
            let domain_line = format!("{domain} {{");
            res.push_str(domain_line.as_str());
        } else {
            let domain_line = format!("http://{domain} https://{domain} {{");
            res.push_str(domain_line.as_str());
        }

        let port = self.port;
        if port > -1 {
            let port_entry = format!("\n\treverse_proxy 127.0.0.1:{port}");
            res.push_str(port_entry.as_str());
        }

        for (path, port) in &self.paths {
            let path_entry = format!("\n\treverse_proxy {path} 127.0.0.1:{port}");
            res.push_str(path_entry.as_str());
        }

        res.push_str("\n}");

        res
    }

    fn spawn_dns_proxy(&self, ip: &str) -> Result<Child, std::io::Error> {
        let name = self.domain.trim_end_matches(".local");

        println!(
            "dns-sd args {:#?}",
            ["-P", name, "_http._tcp", "", "80", self.domain.as_str(), ip]
        );

        Command::new("dns-sd")
            .args(["-P", name, "_http._tcp", "", "80", self.domain.as_str(), ip])
            .spawn()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct DNSLocalConfig {
    records: HashMap<String, Record>,
    automatic_https_redirect: bool,
}

impl DNSLocalConfig {
    fn new() -> DNSLocalConfig {
        DNSLocalConfig {
            records: HashMap::new(),
            automatic_https_redirect: true,
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let command = match &cli.command {
        Some(cmd) => cmd,
        None => {
            return;
        }
    };

    match command {
        Commands::Configure => {
            configure();
        }

        Commands::Run => {
            ctrlc::set_handler(move || {
                stop();
            })
            .expect("error setting ctrl c handler");
            start_server();
        }

        Commands::Start => {
            let args: Vec<_> = args().collect();
            let self_exec = &args[0];

            Command::new(self_exec)
                .arg("run")
                .spawn()
                .expect("Failed to run dnslocalctl");
        }

        Commands::Restart => {
            let endpoint = format!("http://{ADDR}/restart");
            reqwest::blocking::get(endpoint).expect("failed to make restart request");
        }

        Commands::Stop => stop(),

        Commands::Add { proxies } => {
            add_proxies(proxies);
            println!("Added proxies successfully");
        }

        Commands::Remove { proxies } => {
            remove_proxies(proxies);
            println!("Removed proxies successfully")
        }
    }
}

fn parse_proxy_entry(entry: &String) -> (&str, i32, Option<&str>) {
    let parts: Vec<_> = entry.split(':').collect();
    let url = parts[0];
    let port: i32 = parts[1]
        .trim()
        .parse()
        .expect("port part should be a number");

    let url_parts: Vec<_> = url.splitn(2, '/').collect();
    let domain = url_parts[0];
    let path = match url_parts.get(1) {
        Some(value) => Some(*value),
        None => None,
    };

    return (domain, port, path);
}

fn add_proxies(entries: &Vec<String>) {
    let mut config = get_config();

    for entry in entries {
        let (domain, port, path) = parse_proxy_entry(entry);

        let existing_entry = config.records.get_mut(domain);

        let (port, mut paths): (i32, Vec<(String, i32)>) = match path {
            Some(rest) => (-1, vec![(format!("/{rest}"), port)]),

            None => (port, vec![]),
        };

        match existing_entry {
            Some(config) => {
                if paths.is_empty() {
                    // port changed
                    config.port = port
                } else {
                    // removes previous entries of this path
                    config.paths.retain(|it| it.0 != paths[0].0);
                    config.paths.append(&mut paths);
                }
            }

            None => {
                let record = Record {
                    domain: domain.to_string(),
                    paths,
                    port,
                };

                config.records.insert(domain.to_string(), record);
            }
        }
    }

    save_config(&config);
}

fn save_config(config: &DNSLocalConfig) {
    let json = serde_json::to_string_pretty(config).expect("failed to serialize config");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("./dnslocal.json")
        .expect("failed to open/create config file");

    file.write(json.as_bytes()).unwrap();
}

fn remove_proxies(entries: &Vec<String>) {
    let mut config = get_config();

    for entry in entries {
        let (domain, port, path) = parse_proxy_entry(entry);
        if let Some(existing) = config.records.get_mut(domain) {
            match path {
                Some(path) => {
                    let path = format!("/{path}");
                    existing.paths.retain(|it| it.0 != path || it.1 != port);

                    if existing.paths.is_empty() && existing.port == -1 {
                        config.records.remove(domain);
                    }
                }

                None => {
                    if port == existing.port {
                        if existing.paths.is_empty() {
                            config.records.remove(domain);
                        } else {
                            existing.port = -1
                        }
                    }
                }
            }
        }
    }

    save_config(&config);
}

fn start_server() {
    let server = Server::http(ADDR).unwrap();
    let mut proxy_processes: Vec<Child> = start();

    for request in server.incoming_requests() {
        println!(
            "[DNSLocal] {} {} {}",
            Local::now(),
            request.method(),
            request.url()
        );

        if request.method() != &Method::Get {
            _ = request.respond(Response::empty(405));
            continue;
        }

        match request.url() {
            "/restart" => {
                let config = get_config();
                restart(&mut proxy_processes, &config);
            }

            "/quit" => {
                quit(&mut proxy_processes);
                break;
            }

            &_ => {}
        }

        _ = request.respond(Response::from_string("ok"));
    }
}

fn stop() {
    let endpoint = format!("http://{ADDR}/quit");
    reqwest::blocking::get(endpoint).expect("failed to make restart request");
}

// [ ] Handle empty records
fn restart(processes: &mut Vec<Child>, config: &DNSLocalConfig) {
    // reload caddy
    update_caddyfile(&config);

    Command::new("caddy")
        .arg("reload")
        .spawn()
        .expect("failed to reload caddy");

    stop_all_dns_proxies(processes);

    let records = &config.records;
    let entries: Vec<Record> = records.values().cloned().collect();
    let mut new_processes = spawn_dns_proxies(&entries);

    processes.append(&mut new_processes);
}

fn start() -> Vec<Child> {
    let config = get_config();

    update_caddyfile(&config);

    Command::new("caddy")
        .arg("start")
        .spawn()
        .expect("failed to start caddy");

    let entries: Vec<Record> = config.records.values().cloned().collect();
    spawn_dns_proxies(&entries)
}

fn spawn_dns_proxies(records: &Vec<Record>) -> Vec<Child> {
    let ip = local_ip().unwrap();

    let mut processes: Vec<Child> = vec![];
    let mut added: HashSet<String> = HashSet::new();
    for record in records.into_iter() {
        if added.contains(&record.domain) {
            continue;
        }

        if let Ok(child) = record.spawn_dns_proxy(ip.to_string().as_str()) {
            processes.push(child);
            added.insert(record.domain.clone());
        } else {
            println!("error spawning dns responder for {}", record.domain);
        }
    }

    processes
}

fn get_config() -> DNSLocalConfig {
    let config = match File::open("./dnslocal.json") {
        Ok(file) => file,
        Err(_) => return DNSLocalConfig::new(),
    };

    let mut config_json = String::new();
    BufReader::new(config)
        .read_to_string(&mut config_json)
        .expect("error reading json string");

    if config_json.is_empty() {
        return DNSLocalConfig::new();
    }

    let config: DNSLocalConfig =
        serde_json::from_str(&config_json).expect("Invalid config structure");

    config
}

fn stop_all_dns_proxies(processes: &mut Vec<Child>) {
    for process in processes.iter_mut() {
        _ = process.kill();
    }

    processes.clear();
}

fn update_caddyfile(config: &DNSLocalConfig) {
    let mut config_content = String::new();
    let records = &config.records;
    for (_, entry) in records.into_iter() {
        config_content.push_str(entry.entry(config.automatic_https_redirect).as_str());
        config_content.push_str("\n")
    }

    config_content.push_str("\n");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open("./Caddyfile")
        .unwrap();

    file.write_all(config_content.as_bytes()).unwrap();
}

fn quit(processes: &mut Vec<Child>) {
    stop_all_dns_proxies(processes);

    // quit caddy
    Command::new("caddy")
        .arg("stop")
        .spawn()
        .expect("failed to reload caddy");
}

fn configure() {
    println!(
        r"
    .___            .__                       .__
  __| _/____   _____|  |   ____   ____ _____  |  |
 / __ |/    \ /  ___/  |  /  _ \_/ ___\\__  \ |  |
/ /_/ |   |  \\___ \|  |_(  <_> )  \___ / __ \|  |__
\____ |___|  /____  >____/\____/ \___  >____  /____/
     \/    \/     \/                 \/     \/
    "
    );
    println!("Configure dnslocalctl to allow server accept requests");
    println!("You may need to grant permissions to trust a local certificate for [local] HTTPS requests.");
    println!("Read more here: https://degreat.co.uk/dnslocal/configure");

    sleep(time::Duration::from_secs(2));

    let mut caddy_server_process = Command::new("caddy")
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    sleep(time::Duration::from_secs(2));

    Command::new("caddy")
        .arg("trust")
        .stdin(Stdio::piped())
        .output()
        .unwrap();

    caddy_server_process.kill().unwrap();
}
