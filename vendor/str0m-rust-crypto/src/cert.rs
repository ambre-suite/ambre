//! Self-signed DTLS certificate, built with RustCrypto alone.

use std::str::FromStr;
use std::time::Duration;

use p256::ecdsa::{DerSignature, SigningKey};
use p256::elliptic_curve::Generate;
use p256::pkcs8::EncodePrivateKey;
use str0m_proto::crypto::dtls::DtlsCert;
use x509_cert::builder::profile::BuilderProfile;
use x509_cert::builder::{Builder, CertificateBuilder};
use x509_cert::der::Encode;
use x509_cert::ext::Extension;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::{SubjectPublicKeyInfo, SubjectPublicKeyInfoRef};
use x509_cert::time::Validity;
use x509_cert::TbsCertificate;

/// A self-signed end-entity certificate with no extension: DTLS peers check only its
/// fingerprint against the one the session description carries.
struct SelfSigned(Name);

impl BuilderProfile for SelfSigned {
    fn get_issuer(&self, subject: &Name) -> Name {
        subject.clone()
    }

    fn get_subject(&self) -> Name {
        self.0.clone()
    }

    fn build_extensions(
        &self,
        _spk: SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: SubjectPublicKeyInfoRef<'_>,
        _tbs: &TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        Ok(Vec::new())
    }
}

#[derive(Debug)]
pub(crate) struct CertError;

const ONE_YEAR: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// An ECDSA P-256 key and a certificate valid for one year, as dimpl's rcgen path made.
pub(crate) fn generate_self_signed() -> Result<DtlsCert, CertError> {
    let mut rng = rand::rng();
    let key = SigningKey::generate_from_rng(&mut rng);
    let subject = Name::from_str("O=DTLS,CN=DTLS Peer").map_err(|_| CertError)?;
    // Firefox refuses two certificates with one serial, across processes as well.
    let serial = SerialNumber::generate(&mut rng);
    let validity = Validity::from_now(ONE_YEAR).map_err(|_| CertError)?;
    let spki = SubjectPublicKeyInfo::from_key(key.verifying_key()).map_err(|_| CertError)?;
    let cert = CertificateBuilder::new(SelfSigned(subject), serial, validity, spki)
        .map_err(|_| CertError)?
        .build_with_rng::<_, DerSignature, _>(&key, &mut rng)
        .map_err(|_| CertError)?;
    Ok(DtlsCert {
        certificate: cert.to_der().map_err(|_| CertError)?,
        private_key: key.to_pkcs8_der().map_err(|_| CertError)?.as_bytes().to_vec(),
    })
}
