/// Redis key for a server's metadata hash.
/// Fields: `ip`, `port`, `zone`, `status`, `players`.
pub fn server_key(id: &str) -> String {
    format!("server:{id}")
}

/// Redis key for the set of active server IDs.
pub fn active_servers_key() -> &'static str {
    "servers:active"
}
