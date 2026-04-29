//! ATLAS enterprise authentication: OIDC, SAML 2.0, and SCIM 2.0 (T7.3).
//!
//! Integrates with any standards-compliant identity provider (IdP):
//!
//! - **OIDC** ([`oidc`]): OAuth 2.0 + OpenID Connect authorization-code
//!   flow, token introspection, and JWKS verification.  Works with Okta,
//!   Azure AD, Auth0, Keycloak, and Google Workspace.
//!
//! - **SAML 2.0** ([`saml`]): SP-initiated SSO with assertion signature
//!   verification; attribute mapping from the IdP assertion to ATLAS
//!   principals and groups.
//!
//! - **SCIM 2.0** ([`scim`]): server-side SCIM endpoint for automated
//!   user and group provisioning / deprovisioning.  Integrates with
//!   Okta SCIM, Azure AD SCIM, and OneLogin.
//!
//! - **Session management** ([`session`]): short-lived session tokens
//!   issued after successful OIDC/SAML login; maps external identities
//!   to ATLAS capability-token principals.

pub mod oidc;
pub mod saml;
pub mod scim;
pub mod session;

pub use oidc::{OidcConfig, OidcError, TokenClaims};
pub use saml::{SamlAssertion, SamlConfig, SamlError};
pub use scim::{ScimGroup, ScimServer, ScimUser};
pub use session::{AuthSession, SessionStore};
