//! AWS Signature Version 4 for S3 requests (header authorization), from the public
//! specification: <https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html>.

use std::fmt::Write;

use sha2::{Digest, Sha256};

/// SHA-256 of an empty payload, as S3 wants it for bodiless requests.
pub(crate) const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// HMAC-SHA256 (RFC 2104).
pub(crate) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        block.iter_mut().zip(Sha256::digest(key)).for_each(|(b, k)| *b = k);
    } else {
        block.iter_mut().zip(key).for_each(|(b, k)| *b = *k);
    }
    let inner = Sha256::new().chain_update(block.map(|b| b ^ 0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(block.map(|b| b ^ 0x5c)).chain_update(inner).finalize().into()
}

/// An object key as a URI path: every byte but the unreserved characters and `/` percent-encoded,
/// the form S3 both expects in the request and signs.
pub(crate) fn encode_path(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

pub(crate) struct Signer<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub region: &'a str,
}

impl Signer<'_> {
    /// The `Authorization` header for one request with no query string. `path` is already
    /// encoded ([`encode_path`]); `headers` are every header to sign besides `host`, names in
    /// lower case, `x-amz-date` (`amz_date`) and `x-amz-content-sha256` (`payload_sha256`) among
    /// them.
    pub fn authorization(&self, method: &str, host: &str, path: &str, amz_date: &str, headers: &[(&str, &str)], payload_sha256: &str) -> String {
        let date = amz_date.get(..8).unwrap_or(amz_date);
        let mut signed: Vec<(&str, &str)> = Vec::with_capacity(headers.len() + 1);
        signed.push(("host", host));
        signed.extend_from_slice(headers);
        signed.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let names = signed.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(";");
        let canonical_headers: String = signed.iter().map(|(name, value)| format!("{name}:{}\n", value.trim())).collect();
        let canonical = format!("{method}\n{path}\n\n{canonical_headers}\n{names}\n{payload_sha256}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));
        let mut key = hmac_sha256(format!("AWS4{}", self.secret_access_key).as_bytes(), date.as_bytes());
        for part in [self.region, "s3", "aws4_request"] {
            key = hmac_sha256(&key, part.as_bytes());
        }
        let signature = hex(&hmac_sha256(&key, to_sign.as_bytes()));
        format!("AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={names},Signature={signature}", self.access_key_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2: a key shorter than the block.
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // Test case 6: a key longer than the block is hashed first.
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    /// The worked examples in AWS's SigV4 header-authorization documentation.
    const EXAMPLE: Signer<'static> =
        Signer { access_key_id: "AKIAIOSFODNN7EXAMPLE", secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", region: "us-east-1" };

    #[test]
    fn get_object_matches_the_aws_example() {
        let auth = EXAMPLE.authorization(
            "GET",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            "20130524T000000Z",
            &[("range", "bytes=0-9"), ("x-amz-content-sha256", EMPTY_SHA256), ("x-amz-date", "20130524T000000Z")],
            EMPTY_SHA256,
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn put_object_matches_the_aws_example() {
        let payload = sha256_hex(b"Welcome to Amazon S3.");
        assert_eq!(payload, "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
        let path = format!("/{}", encode_path("test$file.text"));
        assert_eq!(path, "/test%24file.text");
        let auth = EXAMPLE.authorization(
            "PUT",
            "examplebucket.s3.amazonaws.com",
            &path,
            "20130524T000000Z",
            &[
                ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("x-amz-date", "20130524T000000Z"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                ("x-amz-content-sha256", &payload),
            ],
            &payload,
        );
        assert!(auth.ends_with(
            "SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class,\
             Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        ));
    }

    #[test]
    fn keys_keep_slashes_and_unreserved_characters() {
        assert_eq!(encode_path("pcraft/tiles/ab12.zst"), "pcraft/tiles/ab12.zst");
        assert_eq!(encode_path("a b+c~d_e-f.g/é"), "a%20b%2Bc~d_e-f.g/%C3%A9");
    }
}
