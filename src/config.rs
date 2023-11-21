use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
};

use serde::{Deserialize, Serialize};

use crate::record::Record;

#[derive(Debug, Deserialize, Serialize)]
pub struct DotLocalConfig {
    pub records: HashMap<String, Record>,
    pub automatic_https_redirect: bool,
    pub lan_enabled: bool,
}

impl DotLocalConfig {
    pub fn new() -> DotLocalConfig {
        DotLocalConfig {
            records: HashMap::new(),
            automatic_https_redirect: true,
            lan_enabled: true,
        }
    }

    pub fn records_list(&self) -> Vec<Record> {
        let records = &self.records;
        let entries: Vec<Record> = records.values().cloned().collect();

        entries
    }

    pub fn get() -> DotLocalConfig {
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
}

// Mark: Methods

impl DotLocalConfig {
    pub fn add_proxies(&mut self, entries: &Vec<String>) {
        for entry in entries {
            let (domain, port, path) = DotLocalConfig::parse_proxy_entry(entry);

            let existing_entry = self.records.get_mut(domain);

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

                    self.records.insert(domain.to_string(), record);
                }
            }
        }

        self.save();
    }

    pub fn remove_proxies(&self, entries: &Vec<String>) {
        let mut config = DotLocalConfig::get();

        for entry in entries {
            let (domain, port, path) = DotLocalConfig::parse_proxy_entry(entry);
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

        self.save();
    }

    pub fn remove_all_proxies(&self) {
        let mut config = DotLocalConfig::get();
        config.records = HashMap::new();

        self.save();
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

    pub fn save(&self) {
        let json = serde_json::to_string_pretty(self).expect("failed to serialize config");

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("./dotlocal.json")
            .expect("failed to open/create config file");

        file.write(json.as_bytes()).unwrap();
    }
}
