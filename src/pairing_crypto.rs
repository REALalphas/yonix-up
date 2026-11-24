//! Small, single-use cryptographic channel for pairing-code authentication.
//!
//! The pairing code is only an SPAKE2 password. This module never broadcasts it or a
//! derivative of it. The resulting channel permits exactly one request and
//! one response, each protected with a distinct nonce.

use anyhow::{Result, anyhow, bail};
use ring::{aead, hmac};
use spake2::{Ed25519Group, Identity, Password, Spake2};

const ID_A: &[u8] = b"yonix-pairing-joiner-v1";
const ID_B: &[u8] = b"yonix-pairing-initiator-v1";
const PROOF: &[u8] = b"yonix-pairing-server-proof-v1";
const NONCE_REQUEST: [u8; 12] = *b"yonix-req-01";
const NONCE_REPLY: [u8; 12] = *b"yonix-reply1";

pub struct Join {
    state: Spake2<Ed25519Group>,
}

pub struct Channel {
    key: [u8; 32],
    request_used: bool,
    reply_used: bool,
}

pub fn start_join(code: &str) -> Result<(Join, String)> {
    let password = Password::new(code.as_bytes());
    let (state, msg) =
        Spake2::<Ed25519Group>::start_a(&password, &Identity::new(ID_A), &Identity::new(ID_B));
    Ok((Join { state }, crate::pairing::encode_b64url(&msg)))
}

pub fn start_share(
    code: &str,
    join_msg: &str,
    invite_id: &str,
) -> Result<(Channel, String, String)> {
    let inbound = crate::pairing::decode_b64url(join_msg)?;
    let password = Password::new(code.as_bytes());
    let (state, msg) =
        Spake2::<Ed25519Group>::start_b(&password, &Identity::new(ID_A), &Identity::new(ID_B));
    let key = state
        .finish(&inbound)
        .map_err(|_| anyhow!("invalid pairing message"))?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| anyhow!("unexpected SPAKE2 key length"))?;
    let proof = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, &key),
        &[PROOF, &msg, invite_id.as_bytes()].concat(),
    );
    Ok((
        Channel {
            key: channel_key(&key),
            request_used: false,
            reply_used: false,
        },
        crate::pairing::encode_b64url(&msg),
        crate::pairing::encode_b64url(proof.as_ref()),
    ))
}

impl Join {
    pub fn finish(self, msg: &str, proof: &str, invite_id: &str) -> Result<Channel> {
        let inbound = crate::pairing::decode_b64url(msg)?;
        let key = self
            .state
            .finish(&inbound)
            .map_err(|_| anyhow!("invalid pairing message"))?;
        let key: [u8; 32] = key
            .try_into()
            .map_err(|_| anyhow!("unexpected SPAKE2 key length"))?;
        let supplied = crate::pairing::decode_b64url(proof)?;
        if hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, &key),
            &[PROOF, &inbound, invite_id.as_bytes()].concat(),
            &supplied,
        )
        .is_err()
        {
            bail!("invalid server proof")
        }
        Ok(Channel {
            key: channel_key(&key),
            request_used: false,
            reply_used: false,
        })
    }
}

fn channel_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut key = [0; 32];
    key.copy_from_slice(
        hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, shared),
            b"yonix-pairing-encryption-v1",
        )
        .as_ref(),
    );
    key
}

impl Channel {
    fn crypt(&self, data: &[u8], nonce: [u8; 12], seal: bool) -> Result<Vec<u8>> {
        let key = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &self.key)
            .map_err(|_| anyhow!("invalid channel key"))?;
        let key = aead::LessSafeKey::new(key);
        let nonce = aead::Nonce::assume_unique_for_key(nonce);
        if seal {
            let mut out = data.to_vec();
            key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut out)
                .map_err(|_| anyhow!("encryption failed"))?;
            Ok(out)
        } else {
            let mut out = data.to_vec();
            Ok(key
                .open_in_place(nonce, aead::Aad::empty(), &mut out)
                .map_err(|_| anyhow!("decryption failed"))?
                .to_vec())
        }
    }
    pub fn seal_request(&mut self, data: &[u8]) -> Result<String> {
        if self.request_used {
            bail!("request already sealed")
        }
        self.request_used = true;
        Ok(crate::pairing::encode_b64url(&self.crypt(
            data,
            NONCE_REQUEST,
            true,
        )?))
    }
    pub fn open_request(&mut self, wire: &str) -> Result<Vec<u8>> {
        if self.request_used {
            bail!("request already opened")
        }
        self.request_used = true;
        let b = crate::pairing::decode_b64url(wire)?;
        self.crypt(&b, NONCE_REQUEST, false)
    }
    pub fn seal_reply(&mut self, data: &[u8]) -> Result<String> {
        if self.reply_used {
            bail!("reply already sealed")
        }
        self.reply_used = true;
        Ok(crate::pairing::encode_b64url(&self.crypt(
            data,
            NONCE_REPLY,
            true,
        )?))
    }
    pub fn open_reply(&mut self, wire: &str) -> Result<Vec<u8>> {
        if self.reply_used {
            bail!("reply already opened")
        }
        self.reply_used = true;
        let b = crate::pairing::decode_b64url(wire)?;
        self.crypt(&b, NONCE_REPLY, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_proof() {
        let (join, first) = start_join("123456").unwrap();
        let (mut server, second, proof) = start_share("123456", &first, "invite").unwrap();
        let mut client = join.finish(&second, &proof, "invite").unwrap();
        let wire = client.seal_request(b"hello").unwrap();
        assert_eq!(server.open_request(&wire).unwrap(), b"hello");
        let wire = server.seal_reply(b"ok").unwrap();
        assert_eq!(client.open_reply(&wire).unwrap(), b"ok");
    }
    #[test]
    fn wrong_pin_fails() {
        let (join, first) = start_join("123456").unwrap();
        let (_server, second, proof) = start_share("654321", &first, "invite").unwrap();
        assert!(join.finish(&second, &proof, "invite").is_err());
    }

    #[test]
    fn channel_messages_are_single_use_and_tampering_is_rejected() {
        let (join, first) = start_join("123456").unwrap();
        let (mut server, second, proof) = start_share("123456", &first, "invite").unwrap();
        let mut client = join.finish(&second, &proof, "invite").unwrap();

        let request = client.seal_request(b"credentials").unwrap();
        assert_eq!(server.open_request(&request).unwrap(), b"credentials");
        assert!(server.open_request(&request).is_err());

        let response = server.seal_reply(b"accepted").unwrap();
        assert_eq!(client.open_reply(&response).unwrap(), b"accepted");
        assert!(client.open_reply(&response).is_err());

        // A modified ciphertext must not be accepted as a valid credentials
        // message. The channel still consumes the request after this attempt,
        // preventing a captured exchange from being retried.
        let (join, first) = start_join("123456").unwrap();
        let (mut server, second, proof) = start_share("123456", &first, "invite").unwrap();
        let mut client = join.finish(&second, &proof, "invite").unwrap();
        let request = client.seal_request(b"credentials").unwrap();
        let mut bytes = crate::pairing::decode_b64url(&request).unwrap();
        bytes[0] ^= 1;
        let tampered = crate::pairing::encode_b64url(&bytes);
        assert!(server.open_request(&tampered).is_err());
        assert!(server.open_request(&request).is_err());
    }
}
