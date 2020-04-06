// Copyright 2015-2020 Capital One Services, LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::errors;
use crate::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use wascap::jwt::Token;
use wascap::prelude::*;

lazy_static! {
    pub(crate) static ref CLAIMS: Arc<RwLock<HashMap<String, Claims<wascap::jwt::Actor>>>> =
        { Arc::new(RwLock::new(HashMap::new())) };
    pub(crate) static ref WAPC_GUEST_MAP: Arc<RwLock<HashMap<u64, String>>> =
        { Arc::new(RwLock::new(HashMap::new())) };
    static ref AUTH_HOOK: RwLock<Option<Box<AuthHook>>> = RwLock::new(None);
}

type AuthHook = dyn Fn(&Token<wascap::jwt::Actor>) -> bool + Sync + Send + 'static;

#[allow(dead_code)]
pub(crate) fn set_auth_hook<F>(hook: F)
where
    F: Fn(&Token<wascap::jwt::Actor>) -> bool + Sync + Send + 'static,
{
    *AUTH_HOOK.write().unwrap() = Some(Box::new(hook))
}

pub(crate) fn get_all_claims() -> Vec<(String, Claims<wascap::jwt::Actor>)> {
    CLAIMS
        .read()
        .unwrap()
        .iter()
        .map(|(pk, claims)| (pk.clone(), claims.clone()))
        .collect()
}

pub(crate) fn check_auth(token: &Token<wascap::jwt::Actor>) -> bool {
    let lock = AUTH_HOOK.read().unwrap();
    match *lock {
        Some(ref f) => f(token),
        None => true,
    }
}

pub(crate) fn can_id_invoke(id: u64, capability_id: &str) -> bool {
    WAPC_GUEST_MAP
        .read()
        .unwrap()
        .get(&id)
        .map_or(false, |pk| can_invoke(pk, capability_id))
}

pub(crate) fn pk_for_id(id: u64) -> String {
    WAPC_GUEST_MAP
        .read()
        .unwrap()
        .get(&id)
        .map_or(format!("actor:{}", id), |s| s.clone())
}

pub(crate) fn can_invoke(pk: &str, capability_id: &str) -> bool {
    if pk == capability_id {
        return true;
    }
    CLAIMS.read().unwrap().get(pk).map_or(false, |claims| {
        claims
            .metadata
            .as_ref()
            .unwrap()
            .caps
            .as_ref()
            .map_or(false, |caps| caps.contains(&capability_id.to_string()))
    })
}

pub(crate) fn get_claims(pk: &str) -> Option<Claims<wascap::jwt::Actor>> {
    CLAIMS.read().unwrap().get(pk).cloned()
}

// Extract claims from the JWT embedded in the wasm module's custom section
pub(crate) fn extract_claims(buf: &[u8]) -> Result<wascap::jwt::Token<wascap::jwt::Actor>> {
    let token = wascap::wasm::extract_claims(buf)?;
    match token {
        Some(token) => {
            enforce_validation(&token.jwt)?; // returns an `Err` if validation fails
            if !check_auth(&token) {
                // invoke the auth hook, if there is one
                return Err(errors::new(errors::ErrorKind::Authorization(
                    "Authorization hook denied access to module".into(),
                )));
            }

            info!(
                "Discovered capability attestations: {}",
                token
                    .claims
                    .metadata
                    .as_ref()
                    .unwrap()
                    .caps
                    .clone()
                    .unwrap()
                    .join(",")
            );
            Ok(token)
        }
        None => Err(errors::new(errors::ErrorKind::Authorization(
            "No embedded JWT in actor module".to_string(),
        ))),
    }
}

fn enforce_validation(jwt: &str) -> Result<()> {
    let v = validate_token::<wascap::jwt::Actor>(jwt)?;
    if v.expired {
        Err(errors::new(errors::ErrorKind::Authorization(
            "Expired token".to_string(),
        )))
    } else if v.cannot_use_yet {
        Err(errors::new(errors::ErrorKind::Authorization(format!(
            "Module cannot be used before {}",
            v.not_before_human
        ))))
    } else {
        Ok(())
    }
}

pub(crate) fn register_claims(guest_id: u64, claims: Claims<wascap::jwt::Actor>) {
    WAPC_GUEST_MAP
        .write()
        .unwrap()
        .insert(guest_id, claims.subject.clone());

    CLAIMS
        .write()
        .unwrap()
        .insert(claims.subject.clone(), claims);
}

pub(crate) fn unregister_claims(guest_id: u64) {
    let pk = pk_for_id(guest_id);

    CLAIMS.write().unwrap().remove(&pk);
    WAPC_GUEST_MAP.write().unwrap().remove(&guest_id);
}
