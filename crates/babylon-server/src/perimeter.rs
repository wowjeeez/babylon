#[must_use]
pub fn dev_no_auth_allowed(bind: &str) -> bool {
    let host = bind.rsplit_once(':').map_or(bind, |(h, _)| h);
    host == "127.0.0.1" || host == "::1" || host == "localhost"
}

#[cfg(test)]
mod tests {
    use super::dev_no_auth_allowed;

    #[test]
    fn dev_guard() {
        assert!(dev_no_auth_allowed("127.0.0.1:8787"));
        assert!(dev_no_auth_allowed("localhost:8787"));
        assert!(dev_no_auth_allowed("::1:8787"));
        assert!(!dev_no_auth_allowed("0.0.0.0:8787"));
        assert!(!dev_no_auth_allowed("10.0.0.5:8787"));
    }
}
