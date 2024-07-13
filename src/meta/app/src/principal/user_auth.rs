// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::str::FromStr;

use databend_common_exception::ErrorCode;
use databend_common_exception::Result;
use sha2::Digest;
use sha2::Sha256;

const NO_PASSWORD_STR: &str = "no_password";
const SHA256_PASSWORD_STR: &str = "sha256_password";
const DOUBLE_SHA1_PASSWORD_STR: &str = "double_sha1_password";
const JWT_AUTH_STR: &str = "jwt";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq)]
pub enum AuthType {
    NoPassword,
    Sha256Password,
    DoubleSha1Password,
    JWT,
}

impl FromStr for AuthType {
    type Err = ErrorCode;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            SHA256_PASSWORD_STR => Ok(AuthType::Sha256Password),
            DOUBLE_SHA1_PASSWORD_STR => Ok(AuthType::DoubleSha1Password),
            NO_PASSWORD_STR => Ok(AuthType::NoPassword),
            JWT_AUTH_STR => Ok(AuthType::JWT),
            _ => Err(ErrorCode::InvalidAuthInfo(AuthType::bad_auth_types(s))),
        }
    }
}

impl AuthType {
    pub fn to_str(&self) -> &str {
        match self {
            AuthType::NoPassword => NO_PASSWORD_STR,
            AuthType::Sha256Password => SHA256_PASSWORD_STR,
            AuthType::DoubleSha1Password => DOUBLE_SHA1_PASSWORD_STR,
            AuthType::JWT => JWT_AUTH_STR,
        }
    }

    fn bad_auth_types(s: &str) -> String {
        let all = [
            NO_PASSWORD_STR,
            SHA256_PASSWORD_STR,
            DOUBLE_SHA1_PASSWORD_STR,
            JWT_AUTH_STR,
        ];
        let all = all
            .iter()
            .map(|s| format!("'{}'", s))
            .collect::<Vec<_>>()
            .join("|");
        format!("Expected auth type {}, found: {}", all, s)
    }

    pub fn get_password_type(self) -> Option<PasswordHashMethod> {
        match self {
            AuthType::Sha256Password => Some(PasswordHashMethod::Sha256),
            AuthType::DoubleSha1Password => Some(PasswordHashMethod::DoubleSha1),
            _ => None,
        }
    }
}

impl From<databend_common_ast::ast::AuthType> for AuthType {
    fn from(t: databend_common_ast::ast::AuthType) -> Self {
        match t {
            databend_common_ast::ast::AuthType::NoPassword => AuthType::NoPassword,
            databend_common_ast::ast::AuthType::Sha256Password => AuthType::Sha256Password,
            databend_common_ast::ast::AuthType::DoubleSha1Password => AuthType::DoubleSha1Password,
            databend_common_ast::ast::AuthType::JWT => AuthType::JWT,
        }
    }
}

#[derive(
    serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Default,
)]
pub enum AuthInfo {
    #[default]
    None,
    Password {
        hash_value: Vec<u8>,
        hash_method: PasswordHashMethod,
        need_change: bool,
    },
    JWT,
}

fn calc_sha1(v: &[u8]) -> [u8; 20] {
    let mut m = ::sha1::Sha1::new();
    m.update(v);
    m.finalize().into()
}

fn double_sha1(v: &[u8]) -> [u8; 20] {
    calc_sha1(&calc_sha1(v)[..])
}

impl AuthInfo {
    pub fn new(
        auth_type: AuthType,
        auth_string: &Option<String>,
        need_change: bool,
    ) -> Result<AuthInfo> {
        match auth_type {
            AuthType::NoPassword => Ok(AuthInfo::None),
            AuthType::JWT => Ok(AuthInfo::JWT),
            AuthType::Sha256Password | AuthType::DoubleSha1Password => match auth_string {
                Some(p) => {
                    let method = auth_type.get_password_type().unwrap();
                    Ok(AuthInfo::Password {
                        hash_value: method.hash(p.as_bytes()),
                        hash_method: method,
                        need_change,
                    })
                }
                None => Err(ErrorCode::InvalidAuthInfo("need password".to_string())),
            },
        }
    }

    pub fn create(auth_type: &Option<String>, auth_string: &Option<String>) -> Result<AuthInfo> {
        let default = AuthType::DoubleSha1Password;
        let auth_type = auth_type
            .clone()
            .map(|s| AuthType::from_str(&s))
            .transpose()?
            .unwrap_or(default);
        AuthInfo::new(auth_type, auth_string, false)
    }

    pub fn create2(
        auth_type: &Option<AuthType>,
        auth_string: &Option<String>,
        need_change: bool,
    ) -> Result<AuthInfo> {
        let default = AuthType::DoubleSha1Password;
        let auth_type = auth_type.clone().unwrap_or(default);
        AuthInfo::new(auth_type, auth_string, need_change)
    }

    pub fn alter(
        &self,
        auth_type: &Option<String>,
        auth_string: &Option<String>,
    ) -> Result<AuthInfo> {
        let old_auth_type = self.get_type();
        let new_auth_type = auth_type
            .clone()
            .map(|s| AuthType::from_str(&s))
            .transpose()?
            .unwrap_or(old_auth_type);
        AuthInfo::new(new_auth_type, auth_string, false)
    }

    pub fn alter2(
        &self,
        auth_type: &Option<AuthType>,
        auth_string: &Option<String>,
        need_change: bool,
    ) -> Result<AuthInfo> {
        let old_auth_type = self.get_type();
        let new_auth_type = auth_type.clone().unwrap_or(old_auth_type);

        AuthInfo::new(new_auth_type, auth_string, need_change)
    }

    pub fn get_type(&self) -> AuthType {
        match self {
            AuthInfo::None => AuthType::NoPassword,
            AuthInfo::JWT => AuthType::JWT,
            AuthInfo::Password { hash_method: t, .. } => match t {
                PasswordHashMethod::Sha256 => AuthType::Sha256Password,
                PasswordHashMethod::DoubleSha1 => AuthType::DoubleSha1Password,
            },
        }
    }

    pub fn get_need_change(&self) -> bool {
        match self {
            AuthInfo::None => false,
            AuthInfo::JWT => false,
            AuthInfo::Password { need_change, .. } => *need_change,
        }
    }

    pub fn get_auth_string(&self) -> String {
        match self {
            AuthInfo::Password {
                hash_value: p,
                hash_method: t,
                ..
            } => t.to_string(p),
            AuthInfo::None | AuthInfo::JWT => "".to_string(),
        }
    }

    pub fn get_password(&self) -> Option<Vec<u8>> {
        match self {
            AuthInfo::Password {
                hash_value: p,
                hash_method: _,
                ..
            } => Some(p.to_vec()),
            _ => None,
        }
    }

    pub fn get_password_type(&self) -> Option<PasswordHashMethod> {
        match self {
            AuthInfo::Password {
                hash_value: _,
                hash_method: t,
                ..
            } => Some(*t),
            _ => None,
        }
    }

    fn restore_sha1_mysql(salt: &[u8], input: &[u8], user_password_hash: &[u8]) -> Result<Vec<u8>> {
        // SHA1( password ) XOR SHA1( "20-bytes random data from server" <concat> SHA1( SHA1( password ) ) )
        let mut m = sha1::Sha1::new();
        m.update(salt);
        m.update(user_password_hash);

        let result: [u8; 20] = m.finalize().into();
        if input.len() != result.len() {
            return Err(ErrorCode::SHA1CheckFailed("SHA1 check failed"));
        }
        let mut s = Vec::with_capacity(result.len());
        for i in 0..result.len() {
            s.push(input[i] ^ result[i]);
        }
        Ok(s)
    }

    pub fn auth_mysql(&self, password_input: &[u8], salt: &[u8]) -> Result<bool> {
        match self {
            AuthInfo::None => Ok(true),
            AuthInfo::Password {
                hash_value: p,
                hash_method: t,
                ..
            } => match t {
                PasswordHashMethod::DoubleSha1 => {
                    let password_sha1 = AuthInfo::restore_sha1_mysql(salt, password_input, p)?;
                    Ok(*p == calc_sha1(&password_sha1))
                }
                PasswordHashMethod::Sha256 => Err(ErrorCode::AuthenticateFailure(
                    "login with sha256_password user for mysql protocol not supported yet.",
                )),
            },
            _ => Err(ErrorCode::AuthenticateFailure(format!(
                "user require auth type {}",
                self.get_type().to_str()
            ))),
        }
    }
}

#[derive(
    serde::Serialize,
    serde::Deserialize,
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    num_derive::FromPrimitive,
    Default,
)]
pub enum PasswordHashMethod {
    DoubleSha1 = 1,
    #[default]
    Sha256 = 2,
}

impl PasswordHashMethod {
    pub fn hash(self, user_input: &[u8]) -> Vec<u8> {
        match self {
            PasswordHashMethod::DoubleSha1 => double_sha1(user_input).to_vec(),
            PasswordHashMethod::Sha256 => Sha256::digest(user_input).to_vec(),
        }
    }

    fn to_string(self, hash_value: &[u8]) -> String {
        hex::encode(hash_value)
    }
}
