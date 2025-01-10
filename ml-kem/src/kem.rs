use core::convert::Infallible;
use core::marker::PhantomData;
use hybrid_array::typenum::U32;
#[cfg(not(feature = "decap_key"))]
use hybrid_array::typenum::U64;
use rand_core::CryptoRngCore;

use crate::crypto::{rand, G, H, J};
use crate::param::{DecapsulationKeySize, EncapsulationKeySize, EncodedCiphertext, KemParams};
use crate::pke::{DecryptionKey, EncryptionKey};
use crate::util::B32;
use crate::{Encoded, EncodedSizeUser};

#[cfg(feature = "zeroize")]
use zeroize::{Zeroize, ZeroizeOnDrop};

// Re-export traits from the `kem` crate
pub use ::kem::{Decapsulate, Encapsulate};

/// A shared key resulting from an ML-KEM transaction
pub(crate) type SharedKey = B32;

#[cfg(not(feature = "decap_key"))]
#[derive(Clone, Debug, PartialEq)]
struct DecapsulationSeed<P>
where
    P: KemParams,
{
    d: B32,
    z: B32,
    _phantom: PhantomData<P>,
}

#[derive(Clone, Debug, PartialEq)]
struct DecapsulationKeyInner<P>
where
    P: KemParams,
{
    dk_pke: DecryptionKey<P>,
    ek: EncapsulationKey<P>,
    z: B32,
}

/// A `DecapsulationKey` provides the ability to generate a new key pair, and decapsulate an
/// encapsulated shared key.
#[cfg(feature = "decap_key")]
#[derive(Clone, Debug, PartialEq)]
pub struct DecapsulationKey<P>
where
    P: KemParams,
{
    key: DecapsulationKeyInner<P>,
}
/// A `DecapsulationKey` provides the ability to generate a new key pair, and decapsulate an
/// encapsulated shared key.
#[cfg(not(feature = "decap_key"))]
#[derive(Clone, Debug, PartialEq)]
pub struct DecapsulationKey<P>
where
    P: KemParams,
{
    key: DecapsulationSeed<P>,
}

#[cfg(feature = "zeroize")]
impl<P> Drop for DecapsulationKeyInner<P>
where
    P: KemParams,
{
    fn drop(&mut self) {
        self.dk_pke.zeroize();
        self.z.zeroize();
    }
}

#[cfg(all(feature = "zeroize", not(feature = "decap_key")))]
impl<P> Drop for DecapsulationSeed<P>
where
    P: KemParams,
{
    fn drop(&mut self) {
        self.d.zeroize();
        self.z.zeroize();
    }
}

#[cfg(feature = "zeroize")]
impl<P> Zeroize for DecapsulationKeyInner<P>
where
    P: KemParams,
{
    fn zeroize(&mut self) {
        self.dk_pke.zeroize();
        self.z.zeroize();
    }
}

#[cfg(all(feature = "zeroize", not(feature = "decap_key")))]
impl<P> Zeroize for DecapsulationSeed<P>
where
    P: KemParams,
{
    fn zeroize(&mut self) {
        self.d.zeroize();
        self.z.zeroize();
    }
}

#[cfg(feature = "zeroize")]
impl<P> Drop for DecapsulationKey<P>
where
    P: KemParams,
{
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

#[cfg(feature = "zeroize")]
impl<P> ZeroizeOnDrop for DecapsulationKeyInner<P> where P: KemParams {}

#[cfg(all(feature = "zeroize", not(feature = "decap_key")))]
impl<P> ZeroizeOnDrop for DecapsulationSeed<P> where P: KemParams {}

#[cfg(feature = "zeroize")]
impl<P> ZeroizeOnDrop for DecapsulationKey<P> where P: KemParams {}

impl<P> EncodedSizeUser for DecapsulationKeyInner<P>
where
    P: KemParams,
{
    type EncodedSize = DecapsulationKeySize<P>;

    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    fn from_bytes(enc: &Encoded<Self>) -> Self {
        let (dk_pke, ek_pke, h, z) = P::split_dk(enc);
        let ek_pke = EncryptionKey::from_bytes(ek_pke);

        // XXX(RLB): The encoding here is redundant, since `h` can be computed from `ek_pke`.
        // Should we verify that the provided `h` value is valid?

        Self {
            dk_pke: DecryptionKey::from_bytes(dk_pke),
            ek: EncapsulationKey {
                ek_pke,
                h: h.clone(),
            },
            z: z.clone(),
        }
    }

    fn as_bytes(&self) -> Encoded<Self> {
        let dk_pke = self.dk_pke.as_bytes();
        let ek = self.ek.as_bytes();
        P::concat_dk(dk_pke, ek, self.ek.h.clone(), self.z.clone())
    }
}

#[cfg(not(feature = "decap_key"))]
impl<P> EncodedSizeUser for DecapsulationSeed<P>
where
    P: KemParams,
{
    type EncodedSize = U64;

    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    fn from_bytes(enc: &Encoded<Self>) -> Self {
        let (d, z) = P::split_seed(enc);

        Self {
            d: d.clone(),
            z: z.clone(),
            _phantom: PhantomData,
        }
    }

    fn as_bytes(&self) -> Encoded<Self> {
        self.d.clone().concat(self.z.clone())
    }
}

impl<P> EncodedSizeUser for DecapsulationKey<P>
where
    P: KemParams,
{
    #[cfg(feature = "decap_key")]
    type EncodedSize = DecapsulationKeySize<P>;
    #[cfg(not(feature = "decap_key"))]
    type EncodedSize = U64;

    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    fn from_bytes(enc: &Encoded<Self>) -> Self {
        #[cfg(feature = "decap_key")]
        {
            Self {
                key: DecapsulationKeyInner::<P>::from_bytes(enc),
            }
        }
        #[cfg(not(feature = "decap_key"))]
        {
            Self {
                key: DecapsulationSeed::<P>::from_bytes(enc),
            }
        }
    }

    fn as_bytes(&self) -> Encoded<Self> {
        self.key.as_bytes()
    }
}

// 0xff if x == y, 0x00 otherwise
fn constant_time_eq(x: u8, y: u8) -> u8 {
    let diff = x ^ y;
    let is_zero = !diff & diff.wrapping_sub(1);
    0u8.wrapping_sub(is_zero >> 7)
}

impl<P> ::kem::Decapsulate<EncodedCiphertext<P>, SharedKey> for DecapsulationKeyInner<P>
where
    P: KemParams,
{
    type Error = Infallible;

    fn decapsulate(
        &self,
        encapsulated_key: &EncodedCiphertext<P>,
    ) -> Result<SharedKey, Self::Error> {
        let mp = self.dk_pke.decrypt(encapsulated_key);
        let (Kp, rp) = G(&[&mp, &self.ek.h]);
        let Kbar = J(&[self.z.as_slice(), encapsulated_key.as_ref()]);
        let cp = self.ek.ek_pke.encrypt(&mp, &rp);

        // Constant-time version of:
        //
        // if cp == *ct {
        //     Kp
        // } else {
        //     Kbar
        // }
        let equal = cp
            .iter()
            .zip(encapsulated_key.iter())
            .map(|(&x, &y)| constant_time_eq(x, y))
            .fold(0xff, |x, y| x & y);
        Ok(Kp
            .iter()
            .zip(Kbar.iter())
            .map(|(x, y)| (equal & x) | (!equal & y))
            .collect())
    }
}

#[cfg(not(feature = "decap_key"))]
impl<P> ::kem::Decapsulate<EncodedCiphertext<P>, SharedKey> for DecapsulationSeed<P>
where
    P: KemParams,
{
    type Error = Infallible;

    fn decapsulate(
        &self,
        encapsulated_key: &EncodedCiphertext<P>,
    ) -> Result<SharedKey, Self::Error> {
        DecapsulationKeyInner::<P>::generate_deterministic(&self.d, &self.z)
            .decapsulate(encapsulated_key)
    }
}

impl<P> ::kem::Decapsulate<EncodedCiphertext<P>, SharedKey> for DecapsulationKey<P>
where
    P: KemParams,
{
    type Error = Infallible;

    fn decapsulate(
        &self,
        encapsulated_key: &EncodedCiphertext<P>,
    ) -> Result<SharedKey, Self::Error> {
        self.key.decapsulate(encapsulated_key)
    }
}

impl<P> DecapsulationKeyInner<P>
where
    P: KemParams,
{
    /// Get the [`EncapsulationKey`] which corresponds to this [`DecapsulationKeyInner`].
    pub fn encapsulation_key(&self) -> EncapsulationKey<P> {
        self.ek.clone()
    }

    #[cfg(feature = "decap_key")]
    pub(crate) fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let d: B32 = rand(rng);
        let z: B32 = rand(rng);
        Self::generate_deterministic(&d, &z)
    }

    #[must_use]
    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    pub(crate) fn generate_deterministic(d: &B32, z: &B32) -> Self {
        let (dk_pke, ek_pke) = DecryptionKey::generate(d);
        let ek = EncapsulationKey::new(ek_pke);
        let z = z.clone();
        Self { dk_pke, ek, z }
    }
}

#[cfg(not(feature = "decap_key"))]
impl<P> DecapsulationSeed<P>
where
    P: KemParams,
{
    /// Get the [`EncapsulationKey`] which corresponds to this [`DecapsulationSeed`].
    #[must_use]
    pub fn encapsulation_key(&self) -> EncapsulationKey<P> {
        DecapsulationKeyInner::<P>::generate_deterministic(&self.d, &self.z)
            .encapsulation_key()
            .clone()
    }

    pub(crate) fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let d: B32 = rand(rng);
        let z: B32 = rand(rng);
        Self {
            d,
            z,
            _phantom: PhantomData,
        }
    }

    #[must_use]
    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    #[cfg(feature = "deterministic")]
    pub(crate) fn generate_deterministic(d: &B32, z: &B32) -> Self {
        Self {
            d: *d,
            z: *z,
            _phantom: PhantomData,
        }
    }
}

impl<P> DecapsulationKey<P>
where
    P: KemParams,
{
    /// Get the [`EncapsulationKey`] which corresponds to this [`DecapsulationKey`].
    #[must_use]
    pub fn encapsulation_key(&self) -> EncapsulationKey<P> {
        self.key.encapsulation_key()
    }

    pub(crate) fn generate(rng: &mut impl CryptoRngCore) -> Self {
        #[cfg(not(feature = "decap_key"))]
        {
            DecapsulationKey {
                key: DecapsulationSeed::<P>::generate(rng),
            }
        }
        #[cfg(feature = "decap_key")]
        {
            DecapsulationKey {
                key: DecapsulationKeyInner::<P>::generate(rng),
            }
        }
    }

    #[must_use]
    #[allow(clippy::similar_names)] // allow dk_pke, ek_pke, following the spec
    #[cfg(feature = "deterministic")]
    pub(crate) fn generate_deterministic(d: &B32, z: &B32) -> Self {
        #[cfg(not(feature = "decap_key"))]
        {
            DecapsulationKey {
                key: DecapsulationSeed::<P>::generate_deterministic(d, z),
            }
        }
        #[cfg(feature = "decap_key")]
        {
            DecapsulationKey {
                key: DecapsulationKeyInner::<P>::generate_deterministic(d, z),
            }
        }
    }
}

/// An `EncapsulationKey` provides the ability to encapsulate a shared key so that it can only be
/// decapsulated by the holder of the corresponding decapsulation key.
#[derive(Clone, Debug, PartialEq)]
pub struct EncapsulationKey<P>
where
    P: KemParams,
{
    ek_pke: EncryptionKey<P>,
    h: B32,
}

impl<P> EncapsulationKey<P>
where
    P: KemParams,
{
    fn new(ek_pke: EncryptionKey<P>) -> Self {
        let h = H(ek_pke.as_bytes());
        Self { ek_pke, h }
    }

    fn encapsulate_deterministic_inner(&self, m: &B32) -> (EncodedCiphertext<P>, SharedKey) {
        let (K, r) = G(&[m, &self.h]);
        let c = self.ek_pke.encrypt(m, &r);
        (c, K)
    }
}

impl<P> EncodedSizeUser for EncapsulationKey<P>
where
    P: KemParams,
{
    type EncodedSize = EncapsulationKeySize<P>;

    fn from_bytes(enc: &Encoded<Self>) -> Self {
        Self::new(EncryptionKey::from_bytes(enc))
    }

    fn as_bytes(&self) -> Encoded<Self> {
        self.ek_pke.as_bytes()
    }
}

impl<P> ::kem::Encapsulate<EncodedCiphertext<P>, SharedKey> for EncapsulationKey<P>
where
    P: KemParams,
{
    type Error = Infallible;

    fn encapsulate(
        &self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(EncodedCiphertext<P>, SharedKey), Self::Error> {
        let m: B32 = rand(rng);
        Ok(self.encapsulate_deterministic_inner(&m))
    }
}

#[cfg(feature = "deterministic")]
impl<P> crate::EncapsulateDeterministic<EncodedCiphertext<P>, SharedKey> for EncapsulationKey<P>
where
    P: KemParams,
{
    type Error = Infallible;

    fn encapsulate_deterministic(
        &self,
        m: &B32,
    ) -> Result<(EncodedCiphertext<P>, SharedKey), Self::Error> {
        Ok(self.encapsulate_deterministic_inner(m))
    }
}

/// An implementation of overall ML-KEM functionality.  Generic over parameter sets, but then ties
/// together all of the other related types and sizes.
pub struct Kem<P>
where
    P: KemParams,
{
    _phantom: PhantomData<P>,
}

impl<P> crate::KemCore for Kem<P>
where
    P: KemParams,
{
    type SharedKeySize = U32;
    type CiphertextSize = P::CiphertextSize;
    type DecapsulationKey = DecapsulationKey<P>;
    type EncapsulationKey = EncapsulationKey<P>;

    /// Generate a new (decapsulation, encapsulation) key pair
    fn generate(rng: &mut impl CryptoRngCore) -> (Self::DecapsulationKey, Self::EncapsulationKey) {
        let dk = Self::DecapsulationKey::generate(rng);
        let ek = dk.encapsulation_key().clone();
        (dk, ek)
    }

    #[cfg(feature = "deterministic")]
    fn generate_deterministic(
        d: &B32,
        z: &B32,
    ) -> (Self::DecapsulationKey, Self::EncapsulationKey) {
        let dk = Self::DecapsulationKey::generate_deterministic(d, z);
        let ek = dk.encapsulation_key().clone();
        (dk, ek)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{MlKem1024Params, MlKem512Params, MlKem768Params};
    use ::kem::{Decapsulate, Encapsulate};

    fn round_trip_test<P>()
    where
        P: KemParams,
    {
        let mut rng = rand::thread_rng();

        let dk = DecapsulationKey::<P>::generate(&mut rng);
        let ek = dk.encapsulation_key();

        let (ct, k_send) = ek.encapsulate(&mut rng).unwrap();
        let k_recv = dk.decapsulate(&ct).unwrap();
        assert_eq!(k_send, k_recv);
    }

    #[test]
    fn round_trip() {
        round_trip_test::<MlKem512Params>();
        round_trip_test::<MlKem768Params>();
        round_trip_test::<MlKem1024Params>();
    }

    fn codec_test<P>()
    where
        P: KemParams,
    {
        let mut rng = rand::thread_rng();
        let dk_original = DecapsulationKey::<P>::generate(&mut rng);
        let ek_original = dk_original.encapsulation_key().clone();

        let dk_encoded = dk_original.as_bytes();
        let dk_decoded = DecapsulationKey::from_bytes(&dk_encoded);
        assert_eq!(dk_original, dk_decoded);

        let ek_encoded = ek_original.as_bytes();
        let ek_decoded = EncapsulationKey::from_bytes(&ek_encoded);
        assert_eq!(ek_original, ek_decoded);
    }

    #[test]
    fn codec() {
        codec_test::<MlKem512Params>();
        codec_test::<MlKem768Params>();
        codec_test::<MlKem1024Params>();
    }
}
