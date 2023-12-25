use core::time;
use std::{
    process::{Child, Command, Stdio},
    thread::sleep,
};

use chrono::Local;
use clap::{Subcommand, ValueEnum};
use tiny_http::{Method, Response, Server};

use crate::{
    config::DotLocalConfig,
    helpers::{spawn_dns_proxies, stop_all_dns_proxies, update_caddyfile},
};

#[derive(Subcommand, Debug)]
pub enum Commands {
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

#[derive(Debug, Clone, ValueEnum)]
pub enum Access {
    Local,
    Lan,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum Https {
    Auto,
    Off,
}

const ADDR: &str = "127.0.0.1:2023";

#[cfg(target_arch = "x86_64")]
const CADDY_BIN: &str = "/usr/local/bin/caddy";

#[cfg(not(target_arch = "x86_64"))]
const CADDY_BIN: &str = "/opt/homebrew/bin/caddy";

impl Commands {
    pub fn exec(&self) {
        match self {
            Commands::Configure => self.configure(),

            Commands::Run => self.run(),

            Commands::Start => self.start(),

            Commands::Restart => self.restart(),

            Commands::Stop => Commands::stop(),

            Commands::Add { proxies } => self.add_proxies(proxies),

            Commands::Remove { proxies } => self.remove_proxies(proxies),

            Commands::RemoveAll => self.remove_all_proxies(),

            Commands::Access { option } => self.change_access(option),

            Commands::Https { option } => self.change_https(option),
        }
    }
}

// Mark: Run

impl Commands {
    pub fn run(&self) {
        ctrlc::set_handler(move || {
            Commands::stop();
        })
        .expect("error setting ctrl c handler");
        self.start_server();
    }

    fn start_server(&self) {
        let server = Server::http(ADDR).unwrap();
        let mut proxy_processes: Vec<Child> = self.start_proxy();

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
                    let config = DotLocalConfig::get();
                    self.restart_proxy(&mut proxy_processes, &config);
                }

                "/quit" => {
                    self.quit(&mut proxy_processes);
                    break;
                }

                &_ => {}
            }

            _ = request.respond(Response::from_string("ok"));
        }
    }

    fn start_proxy(&self) -> Vec<Child> {
        let config = DotLocalConfig::get();

        update_caddyfile(&config);

        // [ ] Accept a verbose flag to show caddy logs

        Command::new(CADDY_BIN)
            .arg("start")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start caddy");

        let dns_processes = spawn_dns_proxies(&config);
        println!("Started proxy successfully");

        dns_processes
    }

    fn quit(&self, processes: &mut Vec<Child>) {
        stop_all_dns_proxies(processes);

        // quit caddy
        Command::new(CADDY_BIN)
            .arg("stop")
            .status()
            .expect("failed to stop caddy");
    }
}

// Mark: Restart

impl Commands {
    pub fn restart(&self) {
        let endpoint = format!("http://{ADDR}/restart");
        reqwest::blocking::get(endpoint).expect("failed to make restart request");
    }
}

// Mark: Add proxies

impl Commands {
    pub fn add_proxies(&self, proxies: &Vec<String>) {
        let mut config = DotLocalConfig::get();
        config.add_proxies(proxies);
        println!("Added proxies successfully");
    }
}

// Mark: Remove proxies

impl Commands {
    pub fn remove_proxies(&self, proxies: &Vec<String>) {
        let config = DotLocalConfig::get();
        config.remove_proxies(proxies);
        println!("Removed proxies successfully");
    }

    pub fn remove_all_proxies(&self) {
        let config = DotLocalConfig::get();
        config.remove_all_proxies();
        println!("Removed all proxies successfully");
    }
}

// Mark: Access

impl Commands {
    pub fn change_access(&self, access: &Access) {
        let mut config = DotLocalConfig::get();
        config.lan_enabled = match access {
            Access::Local => false,
            Access::Lan => true,
        };

        config.save();
    }
}

// Mark: Https

impl Commands {
    pub fn change_https(&self, https: &Https) {
        let mut config = DotLocalConfig::get();
        config.automatic_https_redirect = match https {
            Https::Auto => true,
            Https::Off => false,
        };

        config.save();
    }
}

// Mark: Start

impl Commands {
    pub fn start(&self) {
        Command::new("./dotlocalctl")
            .arg("run")
            .spawn()
            .expect("Failed to run dotlocalctl");
    }
}

// Mark: Stop

impl Commands {
    pub fn stop() {
        let endpoint = format!("http://{ADDR}/quit");
        reqwest::blocking::get(endpoint).expect("failed to make restart request");
    }
}

// Mark: Restart

impl Commands {
    pub fn restart_proxy(&self, processes: &mut Vec<Child>, config: &DotLocalConfig) {
        update_caddyfile(&config);
        stop_all_dns_proxies(processes);

        Command::new(CADDY_BIN)
            .arg("reload")
            .spawn()
            .expect("failed to reload caddy");

        let mut new_processes = spawn_dns_proxies(&config);

        processes.append(&mut new_processes);
    }
}

// Mark: Config

impl Commands {
    pub fn configure(&self) {
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
}
