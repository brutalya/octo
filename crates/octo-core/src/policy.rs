use crate::providers::ProviderKind;
use crate::storage::Space;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("private threads may only use local providers")]
    PrivateNonLocal,
    #[error("attachment must come from the thread's space")]
    AttachmentSpace,
    #[error("working directory must not be inside the data directory")]
    CwdInDataDir,
}

pub fn check_provider(space: Space, provider: ProviderKind) -> Result<(), PolicyError> {
    if space == Space::Private && !provider.is_local() {
        return Err(PolicyError::PrivateNonLocal);
    }
    Ok(())
}

pub fn check_attachment(thread_space: Space, attachment_space: Space) -> Result<(), PolicyError> {
    if thread_space != attachment_space {
        return Err(PolicyError::AttachmentSpace);
    }
    Ok(())
}

/// True if `url` points at this device or the local network: `localhost`, a loopback
/// address, or a private/link-local IP literal. Any other host name or address may be a
/// remote service, so it does not count as local.
pub fn is_local_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    match u.host() {
        Some(url::Host::Domain(d)) => d.trim_end_matches('.').eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => local_v4(ip),
        Some(url::Host::Ipv6(ip)) => {
            let s = ip.segments();
            ip.to_ipv4_mapped().is_some_and(local_v4)
                || ip.is_loopback()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
        }
        None => false,
    }
}

fn local_v4(ip: std::net::Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// Like `check_provider`, and Ollama counts as local only if its URL is.
pub fn check_thread(
    space: Space,
    provider: ProviderKind,
    ollama_url: &str,
) -> Result<(), PolicyError> {
    check_provider(space, provider)?;
    if space == Space::Private && provider == ProviderKind::Ollama && !is_local_url(ollama_url) {
        return Err(PolicyError::PrivateNonLocal);
    }
    Ok(())
}
