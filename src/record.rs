use crate::helpers::get_ip;
use std::process::{Command, Child};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Record {
    pub domain: String,
    pub paths: Vec<(String, i32)>,
    pub port: i32,
}

impl Record {
    pub fn entry(&self, automatic_https_redirect: bool, lan_enabled: bool) -> String {
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

    pub fn spawn_dns_proxy(&self, ip: &str) -> Result<Child, std::io::Error> {
        let name = self.domain.trim_end_matches(".local");

        Command::new("dns-sd")
            .args(["-P", name, "_http._tcp", "", "80", self.domain.as_str(), ip])
            .spawn()
    }
}