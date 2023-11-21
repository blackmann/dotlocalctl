use crate::config::DotLocalConfig;
use local_ip_address::local_ip;
use std::{str::FromStr, fs::OpenOptions, io::Write, process::Child, collections::HashSet};

pub fn get_ip(lan_enabled: bool) -> String {
    if lan_enabled {
        let local_ip_addr = local_ip().unwrap().to_string();
        return local_ip_addr;
    }

    let ip = String::from_str("127.0.0.1").unwrap();
    ip
}

pub fn update_caddyfile(config: &DotLocalConfig) {
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

pub fn spawn_dns_proxies(config: &DotLocalConfig) -> Vec<Child> {
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

pub fn stop_all_dns_proxies(processes: &mut Vec<Child>) {
    for process in processes.iter_mut() {
        _ = process.kill();
    }

    processes.clear();
}
