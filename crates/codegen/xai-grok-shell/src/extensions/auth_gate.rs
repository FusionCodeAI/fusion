use agent_client_protocol as acp;

use crate::auth::{AuthManager, GrokAuth};

/// Require xAI auth from a sync context, accepting tokens in the client-side buffer window.
///
/// Use this only for features that genuinely require a grok.com subscription
/// principal (e.g. cloud sandbox environments). For features that should work
/// with any valid Fusion identity — billing, usage, share — prefer
/// [`require_fusion_auth`], which treats a Fusion API key (the primary auth
/// path) as first-class.
pub(crate) fn require_xai_auth(
    auth_manager: &AuthManager,
    missing_message: &'static str,
    non_xai_message: &'static str,
) -> Result<GrokAuth, acp::Error> {
    let auth = auth_manager
        .current_or_expired()
        .ok_or_else(|| acp::Error::auth_required().data(missing_message))?;
    if !auth.is_xai_auth() {
        return Err(acp::Error::auth_required().data(non_xai_message));
    }
    Ok(auth)
}

/// Require *any* valid Fusion identity — a Fusion API key (primary) or an
/// xAI OAuth / external session token (secondary). This is the gate to use
/// for billing, usage, share, and other features that authenticate against
/// the Fusion gateway rather than requiring a grok.com subscription.
pub(crate) fn require_fusion_auth(
    auth_manager: &AuthManager,
    missing_message: &'static str,
) -> Result<GrokAuth, acp::Error> {
    let auth = auth_manager
        .current_or_expired()
        .ok_or_else(|| acp::Error::auth_required().data(missing_message))?;
    if !auth.is_authenticated() {
        return Err(acp::Error::auth_required().data(missing_message));
    }
    Ok(auth)
}
