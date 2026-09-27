use crate::model::Result;

/// Explicit browser origins, independent of the container's listening address.
pub struct Access(Vec<(String, String)>);

impl Access {
    pub fn new(port: u16, public_origin: Option<&str>) -> Result<Self> {
        let mut origins: Vec<_> = ["127.0.0.1", "localhost"]
            .into_iter()
            .map(|host| {
                let host = if port == 80 {
                    host.into()
                } else {
                    format!("{host}:{port}")
                };
                (format!("http://{host}"), host)
            })
            .collect();
        if let Some(origin) = public_origin.filter(|s| !s.is_empty()) {
            let invalid = || {
                "DISPATCH_PUBLIC_ORIGIN must be an http(s) origin, e.g. https://dispatch.example.com (no path, query, or credentials)".to_owned()
            };
            let uri: ureq::http::Uri = origin.parse().map_err(|_| invalid())?;
            let authority = uri.authority().ok_or_else(invalid)?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || authority.as_str().contains('@')
                || uri.host().is_none_or(str::is_empty)
                || uri
                    .path_and_query()
                    .is_some_and(|p| p.as_str() != "/" && !p.as_str().is_empty())
                || origin.contains('#')
            {
                return Err(invalid());
            }
            origins.push((
                origin.trim_end_matches('/').into(),
                authority.as_str().into(),
            ));
        }
        Ok(Self(origins))
    }

    pub fn allows(&self, host: Option<&str>, origin: Option<&str>) -> bool {
        self.0.iter().any(|(allowed_origin, allowed_host)| {
            host == Some(allowed_host.as_str())
                && origin.is_none_or(|origin| origin == allowed_origin)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Access;

    #[test]
    fn localhost_defaults_reject_foreign_hosts_and_origins() {
        let access = Access::new(8080, None).unwrap();
        assert!(access.allows(Some("localhost:8080"), Some("http://localhost:8080")));
        assert!(access.allows(Some("127.0.0.1:8080"), None));
        assert!(!access.allows(None, None));
        assert!(!access.allows(Some("evil.example:8080"), None));
        assert!(!access.allows(Some("localhost:8080"), Some("https://evil.example")));
        assert!(!access.allows(Some("localhost:8080"), Some("null")));
    }

    #[test]
    fn mapped_port_and_tls_proxy_use_explicit_origins() {
        for origin in ["http://localhost:9090", "https://dispatch.example.com"] {
            let access = Access::new(8080, Some(origin)).unwrap();
            let host = origin.split_once("://").unwrap().1;
            assert!(access.allows(Some(host), Some(origin)));
            assert!(access.allows(Some(host), None));
            assert!(!access.allows(Some(host), Some("http://localhost:8080")));
            assert!(access.allows(Some("127.0.0.1:8080"), None)); // container health check
        }
    }

    #[test]
    fn rejects_urls_that_are_not_origins() {
        for origin in [
            "*",
            "example.com",
            "ftp://example.com",
            "https://a/path",
            "https://a/?q=1",
            "https://user:pass@a",
            "https://a/#fragment",
        ] {
            assert!(Access::new(8080, Some(origin)).is_err(), "{origin}");
        }
        assert!(Access::new(8080, Some("https://dispatch.example.com/"))
            .unwrap()
            .allows(
                Some("dispatch.example.com"),
                Some("https://dispatch.example.com")
            ));
        assert!(Access::new(80, None)
            .unwrap()
            .allows(Some("localhost"), Some("http://localhost")));
    }
}
