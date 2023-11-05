use std::{
    collections::{HashMap, HashSet},
    env::{self, args, current_exe},
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    process::{Child, Command, Stdio},
    str::FromStr,
    thread::sleep,
};

use chrono::Local;
use clap::{Parser, Subcommand, ValueEnum};
use core::time;
use local_ip_address::local_ip;
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Response, Server};

const ADDR: &str = "127.0.0.1:2023";
const CADDY_BIN: &str = "/opt/homebrew/bin/caddy";

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

#[derive(Debug, Clone, ValueEnum)]
enum Access {
    Local,
    Lan
}

#[derive(Debug, Clone, ValueEnum)]
enum Https {
    Auto,
    Off,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Setup dotlocalctl and related tools to be able to serve requests
    Configure,

    /// Run dotlocal server and blocks
    Run,

    /// Start dotlocal server in the background
    Start,

    /// Restarts server
    Restart,

    /// Stop server.
    Stop,

    /// Add a proxy entry in the format `<domain>:<port>`. You can add
    /// multiple records separated by space.
    ///
    /// Eg. `dotlocalctl add adeton.local:3000 mangobase.local:3003`
    Add {
        #[arg()]
        proxies: Vec<String>,
    },

    /// Remove a proxy entry or multiple entries.
    ///
    /// Eg. `dotlocalctl remove adeton.local:3000 mangobase.local:3003`
    Remove {
        #[arg()]
        proxies: Vec<String>,
    },

    /// Removes all proxy entries
    RemoveAll,

    /// Enable access on your local network or just your local machine
    Access {
        #[arg()]
        option: Access,
    },

    /// Switch on/off automatic https redirect
    Https {
        #[arg()]
        option: Https,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Record {
    domain: String,
    paths: Vec<(String, i32)>,
    port: i32,
}

impl Record {
    fn entry(&self, automatic_https_redirect: bool, lan_enabled: bool) -> String {
        let mut res = String::new();
        let ip = get_ip(lan_enabled);

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
            let port_entry = format!("\n\treverse_proxy {ip}:{port}");
            res.push_str(port_entry.as_str());
        }

        for (path, port) in &self.paths {
            let path_entry = format!("\n\treverse_proxy {path} {ip}:{port}");
            res.push_str(path_entry.as_str());
        }

        res.push_str("\n}");

        res
    }

    fn spawn_dns_proxy(&self, ip: &str) -> Result<Child, std::io::Error> {
        let name = self.domain.trim_end_matches(".local");

        Command::new("dns-sd")
            .args(["-P", name, "_http._tcp", "", "80", self.domain.as_str(), ip])
            .spawn()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct DotLocalConfig {
    records: HashMap<String, Record>,
    automatic_https_redirect: bool,
    lan_enabled: bool,
}

impl DotLocalConfig {
    fn new() -> DotLocalConfig {
        DotLocalConfig {
            records: HashMap::new(),
            automatic_https_redirect: true,
            lan_enabled: true,
        }
    }

    fn records_list(&self) -> Vec<Record> {
        let records = &self.records;
        let entries: Vec<Record> = records.values().cloned().collect();

        entries
    }
}

fn main() {
    let exe_path = current_exe().unwrap();
    let exe_dir = exe_path.parent().unwrap();
    env::set_current_dir(exe_dir).expect("failed to run command from its directory");

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
                .expect("Failed to run dotlocalctl");
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
            println!("Removed proxies successfully");
        }

        Commands::RemoveAll => {
            remove_all_proxies();
            println!("Removed all proxy entries");
        }

        Commands::Access { option } => {
            let mut config = get_config();
            config.lan_enabled = match option {
                Access::Local => false,
                Access::Lan => true,
            };

            save_config(&config);
        }

        Commands::Https { option } => {
            let mut config = get_config();
            config.automatic_https_redirect = match option {
                Https::Auto => true,
                Https::Off => false,
            };

            save_config(&config);
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

fn save_config(config: &DotLocalConfig) {
    let json = serde_json::to_string_pretty(config).expect("failed to serialize config");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("./dotlocal.json")
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

fn remove_all_proxies() {
    let mut config = get_config();
    config.records = HashMap::new();

    save_config(&config);
}

fn start_server() {
    let server = Server::http(ADDR).unwrap();
    let mut proxy_processes: Vec<Child> = start();

    for request in server.incoming_requests() {
        println!(
            "[DotLocal] {} {} {}",
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

fn restart(processes: &mut Vec<Child>, config: &DotLocalConfig) {
    update_caddyfile(&config);

    Command::new(CADDY_BIN)
        .arg("reload")
        .spawn()
        .expect("failed to reload caddy");

    stop_all_dns_proxies(processes);

    let mut new_processes = spawn_dns_proxies(&config);

    processes.append(&mut new_processes);
}

fn start() -> Vec<Child> {
    let config = get_config();

    update_caddyfile(&config);

    Command::new(CADDY_BIN)
        .arg("start")
        .spawn()
        .expect("failed to start caddy");

    spawn_dns_proxies(&config)
}

fn get_ip(lan_enabled: bool) -> String {
    if lan_enabled {
        let local_ip_addr = local_ip().unwrap().to_string();
        return local_ip_addr;
    }

    let ip = String::from_str("127.0.0.1").unwrap();
    ip
}

fn spawn_dns_proxies(config: &DotLocalConfig) -> Vec<Child> {
    let ip = get_ip(config.lan_enabled);

    let records = config.records_list();

    let mut processes: Vec<Child> = vec![];
    let mut added: HashSet<String> = HashSet::new();
    for record in records.into_iter() {
        if added.contains(&record.domain) {
            continue;
        }

        if let Ok(child) = record.spawn_dns_proxy(ip.as_str()) {
            processes.push(child);
            added.insert(record.domain.clone());
        } else {
            println!("error spawning dns responder for {}", record.domain);
        }
    }

    processes
}

fn get_config() -> DotLocalConfig {
    let config = match File::open("./dotlocal.json") {
        Ok(file) => file,
        Err(_) => return DotLocalConfig::new(),
    };

    let mut config_json = String::new();
    BufReader::new(config)
        .read_to_string(&mut config_json)
        .expect("error reading json string");

    if config_json.is_empty() {
        return DotLocalConfig::new();
    }

    let config: DotLocalConfig =
        serde_json::from_str(&config_json).expect("Invalid config structure");

    config
}

fn stop_all_dns_proxies(processes: &mut Vec<Child>) {
    for process in processes.iter_mut() {
        _ = process.kill();
    }

    processes.clear();
}

fn update_caddyfile(config: &DotLocalConfig) {
    let mut config_content = String::new();
    let records = &config.records;

    let mut made_entry = false;

    for (_, entry) in records.into_iter() {
        config_content.push_str(
            entry
                .entry(config.automatic_https_redirect, config.lan_enabled)
                .as_str(),
        );
        config_content.push_str("\n");

        made_entry = true;
    }

    if !made_entry {
        // this prevents `caddy` from complaining about EOF
        config_content.push_str("\n");
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("./Caddyfile")
        .unwrap();

    file.write_all(config_content.as_bytes()).unwrap();
}

fn quit(processes: &mut Vec<Child>) {
    stop_all_dns_proxies(processes);

    // quit caddy
    Command::new(CADDY_BIN)
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
    println!("Configure dotlocalctl to allow server accept requests");
    println!("You may need to grant permissions to trust a local certificate for [local] HTTPS requests.");
    println!("Read more here: https://degreat.co.uk/dotlocal/configure");

    sleep(time::Duration::from_secs(2));

    let mut caddy_server_process = Command::new(CADDY_BIN)
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    sleep(time::Duration::from_secs(2));

    Command::new(CADDY_BIN)
        .arg("trust")
        .stdin(Stdio::piped())
        .output()
        .unwrap();

    caddy_server_process.kill().unwrap();
}
