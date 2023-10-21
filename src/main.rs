use std::{
    fs::File,
    io::{BufReader, Read},
    process::{Child, Command}, collections::HashSet,
};

use clap::{Parser, Subcommand};
use local_ip_address::local_ip;
use serde::{Deserialize, Serialize};
use tiny_http::{Response, Server};

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

#[derive(Debug, Deserialize, Serialize)]
struct Record {
    domain: String,
    path: String,
    port: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct DNSLocalConfig {
    records: Vec<Record>,
    automatic_https_redirect: bool,
}

fn main() {
    let cli = Cli::parse();

    let command = match &cli.command {
        Some(cmd) => cmd,
        None => panic!("Error getting command"),
    };

    match command {
        Commands::Run => {
            start_server();
        }

        Commands::Start => {
            Command::new("./dnslocal")
                .arg("run")
                .spawn()
                .expect("Failed to run dnslocalctl");
        }

        Commands::Restart => {
            println!("Restarting server");
        }

        Commands::Stop => {
            println!("Stopping server");
        }

        Commands::Add { proxies } => {
            println!("Adding proxy: {:#?}", proxies);
        }

        Commands::Remove { proxies } => {
            println!("Removing proxies: {:#?}", proxies);
        }
    }
}

fn start_server() {
    let addr = "127.0.0.1:2023";
    let server = Server::http(addr).unwrap();
    let mut proxy_procs: Vec<Child> = start();

    for request in server.incoming_requests() {
        match request.url() {
            "/restart" => {
                restart(&mut proxy_procs);
            }

            "/quit" => quit(&mut proxy_procs),

            &_ => {}
        }

        _ = request.respond(Response::from_string("ok"));
    }
}

// [ ] Handle empty records
fn restart(processes: &mut Vec<Child>) {
    // reload caddy
    Command::new("caddy")
        .arg("reload")
        .spawn()
        .expect("failed to reload caddy");

    remove_all_dns_records(processes);
}

fn start() -> Vec<Child> {
    let config = match File::open("./dnslocal.json") {
        Ok(file) => file,
        Err(_) => return vec![],
    };

    let mut config_json = String::new();
    BufReader::new(config)
        .read_to_string(&mut config_json)
        .expect("error reading json string");

    if config_json.is_empty() {
        return vec![];
    }

    let config: DNSLocalConfig =
        serde_json::from_str(&config_json).expect("Invalid config structure");

    let ip = local_ip().unwrap();
    let mut processes: Vec<Child> = vec![];
    let mut added: HashSet<String> = HashSet::new();
    for record in config.records {
        if added.contains(&record.domain) {
            continue;
        }

        let name = record.domain.trim_end_matches(".local");

        if let Ok(child) = Command::new("dns-sd")
            .args([
                "-P",
                name,
                "_http._tcp",
                "",
                "80",
                record.domain.as_str(),
                ip.to_string().as_str(),
            ])
            .spawn()
        {
            processes.push(child);
            added.insert(record.domain);
        } else {
            println!("error spawning dns responder for {}", record.domain);
        }
    }

    processes
}

fn remove_all_dns_records(processes: &mut Vec<Child>) {
    for process in processes.iter_mut() {
        _ = process.kill();
    }

    processes.clear();
}

fn quit(processes: &mut Vec<Child>) {
    remove_all_dns_records(processes);

    // quit caddy
    Command::new("caddy")
        .arg("stop")
        .spawn()
        .expect("failed to reload caddy");
}
