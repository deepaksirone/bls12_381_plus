//! This module provides an implementation of the BLS12-381 scalar field $\mathbb{F}_q$
//! where `q = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001`

use core::convert::TryFrom;
use core::fmt::{self, Formatter};
use core::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};
use rand_core::RngCore;

use ff::{Field, PrimeField};
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq, CtOption};

#[cfg(feature = "bits")]
use core::convert::TryInto;
use elliptic_curve::{
    bigint::{ArrayEncoding, Encoding, U256, U384, U512},
    consts::{U32, U48, U64},
    generic_array::GenericArray,
    ops::{Invert, Reduce},
    scalar::{FromUintUnchecked, IsHigh},
    ScalarPrimitive,
};

use crate::Bls12381G1;
#[cfg(feature = "bits")]
use ff::{FieldBits, PrimeFieldBits};

use crate::util::{adc, decode_hex_into_slice, mac, sbb};

/// Represents an element of the scalar field $\mathbb{F}_q$ of the BLS12-381 elliptic
/// curve construction.
// The internal representation of this type is four 64-bit unsigned
// integers in little-endian order. `Scalar` values are always in
// Montgomery form; i.e., Scalar(a) = aR mod q, with R = 2^256.
#[derive(Clone, Copy, Eq)]
pub struct Scalar(pub(crate) [u64; 4]);

impl fmt::Debug for Scalar {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:x}", self)
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{:x}", self)
    }
}

impl From<u32> for Scalar {
    fn from(val: u32) -> Self {
        Scalar([val as u64, 0, 0, 0]) * R2
    }
}

impl From<u64> for Scalar {
    fn from(val: u64) -> Scalar {
        Scalar([val, 0, 0, 0]) * R2
    }
}

impl ConstantTimeEq for Scalar {
    fn ct_eq(&self, other: &Self) -> Choice {
        self.0[0].ct_eq(&other.0[0])
            & self.0[1].ct_eq(&other.0[1])
            & self.0[2].ct_eq(&other.0[2])
            & self.0[3].ct_eq(&other.0[3])
    }
}

impl PartialEq for Scalar {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.ct_eq(other))
    }
}

impl ConditionallySelectable for Scalar {
    fn conditional_select(a: &Self, b: &Self, choice: Choice) -> Self {
        Scalar([
            u64::conditional_select(&a.0[0], &b.0[0], choice),
            u64::conditional_select(&a.0[1], &b.0[1], choice),
            u64::conditional_select(&a.0[2], &b.0[2], choice),
            u64::conditional_select(&a.0[3], &b.0[3], choice),
        ])
    }
}

impl zeroize::DefaultIsZeroes for Scalar {}

/// q >> 1 = 39f6d3a994cebea4199cec0404d0ec02a9ded2017fff2dff7fffffff80000000
const HALF_MODULUS: Scalar = Scalar([
    0x7fff_ffff_8000_0000,
    0xa9de_d201_7fff_2dff,
    0x199c_ec04_04d0_ec02,
    0x39f6_d3a9_94ce_bea4,
]);

/// Constant representing the modulus
/// q = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001
const MODULUS: Scalar = Scalar([
    0xffff_ffff_0000_0001,
    0x53bd_a402_fffe_5bfe,
    0x3339_d808_09a1_d805,
    0x73ed_a753_299d_7d48,
]);

/// The modulus as u32 limbs.
#[cfg(all(feature = "bits", not(target_pointer_width = "64")))]
const MODULUS_LIMBS_32: [u32; 8] = [
    0x0000_0001,
    0xffff_ffff,
    0xfffe_5bfe,
    0x53bd_a402,
    0x09a1_d805,
    0x3339_d808,
    0x299d_7d48,
    0x73ed_a753,
];

// The number of bits needed to represent the modulus.
const MODULUS_BITS: u32 = 255;

// GENERATOR = 7 (multiplicative generator of r-1 order, that is also quadratic nonresidue)
const GENERATOR: Scalar = Scalar([
    0x0000_000e_ffff_fff1,
    0x17e3_63d3_0018_9c0f,
    0xff9c_5787_6f84_57b0,
    0x3513_3220_8fc5_a8c4,
]);

impl<'a> Neg for &'a Scalar {
    type Output = Scalar;

    #[inline]
    fn neg(self) -> Scalar {
        self.neg()
    }
}

impl Neg for Scalar {
    type Output = Scalar;

    #[inline]
    fn neg(self) -> Scalar {
        -&self
    }
}

impl<'a, 'b> Sub<&'b Scalar> for &'a Scalar {
    type Output = Scalar;

    #[inline]
    fn sub(self, rhs: &'b Scalar) -> Scalar {
        self.sub(rhs)
    }
}

impl<'a, 'b> Add<&'b Scalar> for &'a Scalar {
    type Output = Scalar;

    #[inline]
    fn add(self, rhs: &'b Scalar) -> Scalar {
        self.add(rhs)
    }
}

impl<'a, 'b> Mul<&'b Scalar> for &'a Scalar {
    type Output = Scalar;

    #[inline]
    fn mul(self, rhs: &'b Scalar) -> Scalar {
        self.mul(rhs)
    }
}

impl_binops_additive!(Scalar, Scalar);
impl_binops_multiplicative!(Scalar, Scalar);

/// INV = -(q^{-1} mod 2^64) mod 2^64
const INV: u64 = 0xffff_fffe_ffff_ffff;

/// R = 2^256 mod q
const R: Scalar = Scalar([
    0x0000_0001_ffff_fffe,
    0x5884_b7fa_0003_4802,
    0x998c_4fef_ecbc_4ff5,
    0x1824_b159_acc5_056f,
]);

/// R^2 = 2^512 mod q
const R2: Scalar = Scalar([
    0xc999_e990_f3f2_9c6d,
    0x2b6c_edcb_8792_5c23,
    0x05d3_1496_7254_398f,
    0x0748_d9d9_9f59_ff11,
]);

/// R^3 = 2^768 mod q
const R3: Scalar = Scalar([
    0xc62c_1807_439b_73af,
    0x1b3e_0d18_8cf0_6990,
    0x73d1_3c71_c7b5_f418,
    0x6e2a_5bb9_c8db_33e9,
]);

/// 2^-1
const TWO_INV: Scalar = Scalar([
    0x0000_0000_ffff_ffff,
    0xac42_5bfd_0001_a401,
    0xccc6_27f7_f65e_27fa,
    0x0c12_58ac_d662_82b7,
]);

// 2^S * t = MODULUS - 1 with t odd
const S: u32 = 32;

/// GENERATOR^t where t * 2^s + 1 = q
/// with t odd. In other words, this
/// is a 2^s root of unity.
///
/// `GENERATOR = 7 mod q` is a generator
/// of the q - 1 order multiplicative
/// subgroup.
const ROOT_OF_UNITY: Scalar = Scalar([
    0xb9b5_8d8c_5f0e_466a,
    0x5b1b_4c80_1819_d7ec,
    0x0af5_3ae3_52a3_1e64,
    0x5bf3_adda_19e9_b27b,
]);

/// ROOT_OF_UNITY^-1
const ROOT_OF_UNITY_INV: Scalar = Scalar([
    0x4256_481a_dcf3_219a,
    0x45f3_7b7f_96b6_cad3,
    0xf9c3_f1d7_5f7a_3b27,
    0x2d2f_c049_658a_fd43,
]);

/// GENERATOR^{2^s} where t * 2^s + 1 = q with t odd.
/// In other words, this is a t root of unity.
const DELTA: Scalar = Scalar([
    0x70e3_10d3_d146_f96a,
    0x4b64_c089_19e2_99e6,
    0x51e1_1418_6a8b_970d,
    0x6185_d066_27c0_67cb,
]);

impl Default for Scalar {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl_serde!(
    Scalar,
    |s: &Scalar| s.to_be_bytes(),
    Scalar::from_be_bytes,
    Scalar::BYTES,
    Scalar::HEX_BYTES
);

impl Scalar {
    /// Bytes to represent this field
    pub const BYTES: usize = 32;
    /// The number of hex bytes needed
    pub(crate) const HEX_BYTES: usize = Self::BYTES * 2;
    /// The additive identity.
    pub const ZERO: Scalar = Scalar([0, 0, 0, 0]);

    /// The multiplicative identity.
    pub const ONE: Scalar = R;

    /// Doubles this field element.
    #[inline]
    pub const fn double(&self) -> Scalar {
        // TODO: This can be achieved more efficiently with a bitshift.
        self.add(self)
    }

    /// Attempts to convert a big-endian byte representation of
    /// a scalar into a `Scalar`, failing if the input is not canonical.
    pub fn from_be_bytes(bytes: &[u8; 32]) -> CtOption<Self> {
        let mut tmp = Scalar([0, 0, 0, 0]);

        tmp.0[3] = u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[0..8]).unwrap());
        tmp.0[2] = u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[8..16]).unwrap());
        tmp.0[1] = u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[16..24]).unwrap());
        tmp.0[0] = u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[24..32]).unwrap());

        // Try to subtract the modulus
        let (_, borrow) = sbb(tmp.0[0], MODULUS.0[0], 0);
        let (_, borrow) = sbb(tmp.0[1], MODULUS.0[1], borrow);
        let (_, borrow) = sbb(tmp.0[2], MODULUS.0[2], borrow);
        let (_, borrow) = sbb(tmp.0[3], MODULUS.0[3], borrow);

        // If the element is smaller than MODULUS then the
        // subtraction will underflow, producing a borrow value
        // of 0xffff...ffff. Otherwise, it'll be zero.
        let is_some = (borrow as u8) & 1;

        // Convert to Montgomery form by computing
        // (a.R^0 * R^2) / R = a.R
        tmp *= &R2;

        CtOption::new(tmp, Choice::from(is_some))
    }

    /// Attempts to convert a little-endian byte representation of
    /// a scalar into a `Scalar`, failing if the input is not canonical.
    pub fn from_le_bytes(bytes: &[u8; 32]) -> CtOption<Self> {
        let mut tmp = Scalar([0, 0, 0, 0]);

        tmp.0[0] = u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[0..8]).unwrap());
        tmp.0[1] = u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[8..16]).unwrap());
        tmp.0[2] = u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[16..24]).unwrap());
        tmp.0[3] = u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[24..32]).unwrap());

        // Try to subtract the modulus
        let (_, borrow) = sbb(tmp.0[0], MODULUS.0[0], 0);
        let (_, borrow) = sbb(tmp.0[1], MODULUS.0[1], borrow);
        let (_, borrow) = sbb(tmp.0[2], MODULUS.0[2], borrow);
        let (_, borrow) = sbb(tmp.0[3], MODULUS.0[3], borrow);

        // If the element is smaller than MODULUS then the
        // subtraction will underflow, producing a borrow value
        // of 0xffff...ffff. Otherwise, it'll be zero.
        let is_some = (borrow as u8) & 1;

        // Convert to Montgomery form by computing
        // (a.R^0 * R^2) / R = a.R
        tmp *= &R2;

        CtOption::new(tmp, Choice::from(is_some))
    }

    /// Converts an element of `Scalar` into a byte representation in
    /// little-endian byte order.
    pub fn to_le_bytes(&self) -> [u8; 32] {
        // Turn into canonical form by computing
        // (a.R) / R = a
        let tmp = Scalar::montgomery_reduce(self.0[0], self.0[1], self.0[2], self.0[3], 0, 0, 0, 0);

        let mut res = [0; 32];
        res[0..8].copy_from_slice(&tmp.0[0].to_le_bytes());
        res[8..16].copy_from_slice(&tmp.0[1].to_le_bytes());
        res[16..24].copy_from_slice(&tmp.0[2].to_le_bytes());
        res[24..32].copy_from_slice(&tmp.0[3].to_le_bytes());

        res
    }

    /// Converts an element of `Scalar` into a byte representation in
    /// big-endian byte order.
    pub fn to_be_bytes(&self) -> [u8; 32] {
        // Turn into canonical form by computing
        // (a.R) / R = a
        let tmp = Scalar::montgomery_reduce(self.0[0], self.0[1], self.0[2], self.0[3], 0, 0, 0, 0);

        let mut res = [0; 32];
        res[0..8].copy_from_slice(&tmp.0[3].to_be_bytes());
        res[8..16].copy_from_slice(&tmp.0[2].to_be_bytes());
        res[16..24].copy_from_slice(&tmp.0[1].to_be_bytes());
        res[24..32].copy_from_slice(&tmp.0[0].to_be_bytes());

        res
    }

    /// Create a new [`Scalar`] from the provided big endian hex string.
    pub fn from_be_hex(hex: &str) -> CtOption<Self> {
        let mut buf = [0u8; Self::BYTES];
        decode_hex_into_slice(&mut buf, hex.as_bytes());
        Self::from_be_bytes(&buf)
    }

    /// Create a new [`Scalar`] from the provided little endian hex string.
    pub fn from_le_hex(hex: &str) -> CtOption<Self> {
        let mut buf = [0u8; Self::BYTES];
        decode_hex_into_slice(&mut buf, hex.as_bytes());
        Self::from_le_bytes(&buf)
    }

    /// Converts a 512-bit little endian integer into
    /// a `Scalar` by reducing by the modulus.
    pub fn from_bytes_wide(bytes: &[u8; 64]) -> Scalar {
        Scalar::from_u512([
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[0..8]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[8..16]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[16..24]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[24..32]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[32..40]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[40..48]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[48..56]).unwrap()),
            u64::from_le_bytes(<[u8; 8]>::try_from(&bytes[56..64]).unwrap()),
        ])
    }

    /// Read from output of a KDF
    pub fn from_okm(bytes: &[u8; 48]) -> Scalar {
        const F_2_192: Scalar = Scalar([
            0x5947_6ebc_41b4_528fu64,
            0xc5a3_0cb2_43fc_c152u64,
            0x2b34_e639_40cc_bd72u64,
            0x1e17_9025_ca24_7088u64,
        ]);
        let d0 = Scalar([
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[16..24]).unwrap()),
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[8..16]).unwrap()),
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[0..8]).unwrap()),
            0,
        ]);
        let d1 = Scalar([
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[40..48]).unwrap()),
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[32..40]).unwrap()),
            u64::from_be_bytes(<[u8; 8]>::try_from(&bytes[24..32]).unwrap()),
            0,
        ]);
        (d0 * R2) * F_2_192 + d1 * R2
    }

    fn from_u512(limbs: [u64; 8]) -> Scalar {
        // We reduce an arbitrary 512-bit number by decomposing it into two 256-bit digits
        // with the higher bits multiplied by 2^256. Thus, we perform two reductions
        //
        // 1. the lower bits are multiplied by R^2, as normal
        // 2. the upper bits are multiplied by R^2 * 2^256 = R^3
        //
        // and computing their sum in the field. It remains to see that arbitrary 256-bit
        // numbers can be placed into Montgomery form safely using the reduction. The
        // reduction works so long as the product is less than R=2^256 multiplied by
        // the modulus. This holds because for any `c` smaller than the modulus, we have
        // that (2^256 - 1)*c is an acceptable product for the reduction. Therefore, the
        // reduction always works so long as `c` is in the field; in this case it is either the
        // constant `R2` or `R3`.
        let d0 = Scalar([limbs[0], limbs[1], limbs[2], limbs[3]]);
        let d1 = Scalar([limbs[4], limbs[5], limbs[6], limbs[7]]);
        // Convert to Montgomery form
        d0 * R2 + d1 * R3
    }

    /// Converts from an integer represented in little endian
    /// into its (congruent) `Scalar` representation.
    pub const fn from_raw(val: [u64; 4]) -> Self {
        (&Scalar(val)).mul(&R2)
    }

    /// Converts this `Scalar` into an integer represented in little endian
    pub const fn to_raw(&self) -> [u64; 4] {
        let tmp = Scalar::montgomery_reduce(self.0[0], self.0[1], self.0[2], self.0[3], 0, 0, 0, 0);
        tmp.0
    }

    /// Squares this element.
    #[inline]
    pub const fn square(&self) -> Scalar {
        let (r1, carry) = mac(0, self.0[0], self.0[1], 0);
        let (r2, carry) = mac(0, self.0[0], self.0[2], carry);
        let (r3, r4) = mac(0, self.0[0], self.0[3], carry);

        let (r3, carry) = mac(r3, self.0[1], self.0[2], 0);
        let (r4, r5) = mac(r4, self.0[1], self.0[3], carry);

        let (r5, r6) = mac(r5, self.0[2], self.0[3], 0);

        let r7 = r6 >> 63;
        let r6 = (r6 << 1) | (r5 >> 63);
        let r5 = (r5 << 1) | (r4 >> 63);
        let r4 = (r4 << 1) | (r3 >> 63);
        let r3 = (r3 << 1) | (r2 >> 63);
        let r2 = (r2 << 1) | (r1 >> 63);
        let r1 = r1 << 1;

        let (r0, carry) = mac(0, self.0[0], self.0[0], 0);
        let (r1, carry) = adc(0, r1, carry);
        let (r2, carry) = mac(r2, self.0[1], self.0[1], carry);
        let (r3, carry) = adc(0, r3, carry);
        let (r4, carry) = mac(r4, self.0[2], self.0[2], carry);
        let (r5, carry) = adc(0, r5, carry);
        let (r6, carry) = mac(r6, self.0[3], self.0[3], carry);
        let (r7, _) = adc(0, r7, carry);

        Scalar::montgomery_reduce(r0, r1, r2, r3, r4, r5, r6, r7)
    }

    /// Exponentiates `self` by `by`, where `by` is a
    /// little-endian order integer exponent.
    pub fn pow(&self, by: &[u64; 4]) -> Self {
        let mut res = Self::ONE;
        for e in by.iter().rev() {
            for i in (0..64).rev() {
                res = res.square();
                let mut tmp = res;
                tmp *= self;
                res.conditional_assign(&tmp, (((*e >> i) & 0x1) as u8).into());
            }
        }
        res
    }

    /// Exponentiates `self` by `by`, where `by` is a
    /// little-endian order integer exponent.
    ///
    /// **This operation is variable time with respect
    /// to the exponent.** If the exponent is fixed,
    /// this operation is effectively constant time.
    pub fn pow_vartime(&self, by: &[u64; 4]) -> Self {
        let mut res = Self::ONE;
        for e in by.iter().rev() {
            for i in (0..64).rev() {
                res = res.square();

                if ((*e >> i) & 1) == 1 {
                    res.mul_assign(self);
                }
            }
        }
        res
    }

    /// Computes the multiplicative inverse of this element,
    /// failing if the element is zero.
    pub fn invert(&self) -> CtOption<Self> {
        #[inline(always)]
        fn square_assign_multi(n: &mut Scalar, num_times: usize) {
            for _ in 0..num_times {
                *n = n.square();
            }
        }
        // found using https://github.com/kwantam/addchain
        let mut t0 = self.square();
        let mut t1 = t0 * self;
        let mut t16 = t0.square();
        let mut t6 = t16.square();
        let mut t5 = t6 * t0;
        t0 = t6 * t16;
        let mut t12 = t5 * t16;
        let mut t2 = t6.square();
        let mut t7 = t5 * t6;
        let mut t15 = t0 * t5;
        let mut t17 = t12.square();
        t1 *= t17;
        let mut t3 = t7 * t2;
        let t8 = t1 * t17;
        let t4 = t8 * t2;
        let t9 = t8 * t7;
        t7 = t4 * t5;
        let t11 = t4 * t17;
        t5 = t9 * t17;
        let t14 = t7 * t15;
        let t13 = t11 * t12;
        t12 = t11 * t17;
        t15 *= &t12;
        t16 *= &t15;
        t3 *= &t16;
        t17 *= &t3;
        t0 *= &t17;
        t6 *= &t0;
        t2 *= &t6;
        square_assign_multi(&mut t0, 8);
        t0 *= &t17;
        square_assign_multi(&mut t0, 9);
        t0 *= &t16;
        square_assign_multi(&mut t0, 9);
        t0 *= &t15;
        square_assign_multi(&mut t0, 9);
        t0 *= &t15;
        square_assign_multi(&mut t0, 7);
        t0 *= &t14;
        square_assign_multi(&mut t0, 7);
        t0 *= &t13;
        square_assign_multi(&mut t0, 10);
        t0 *= &t12;
        square_assign_multi(&mut t0, 9);
        t0 *= &t11;
        square_assign_multi(&mut t0, 8);
        t0 *= &t8;
        square_assign_multi(&mut t0, 8);
        t0 *= self;
        square_assign_multi(&mut t0, 14);
        t0 *= &t9;
        square_assign_multi(&mut t0, 10);
        t0 *= &t8;
        square_assign_multi(&mut t0, 15);
        t0 *= &t7;
        square_assign_multi(&mut t0, 10);
        t0 *= &t6;
        square_assign_multi(&mut t0, 8);
        t0 *= &t5;
        square_assign_multi(&mut t0, 16);
        t0 *= &t3;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 7);
        t0 *= &t4;
        square_assign_multi(&mut t0, 9);
        t0 *= &t2;
        square_assign_multi(&mut t0, 8);
        t0 *= &t3;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 8);
        t0 *= &t3;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 8);
        t0 *= &t2;
        square_assign_multi(&mut t0, 5);
        t0 *= &t1;
        square_assign_multi(&mut t0, 5);
        t0 *= &t1;

        CtOption::new(t0, !self.ct_eq(&Self::ZERO))
    }

    #[inline(always)]
    pub(crate) const fn montgomery_reduce(
        r0: u64,
        r1: u64,
        r2: u64,
        r3: u64,
        r4: u64,
        r5: u64,
        r6: u64,
        r7: u64,
    ) -> Self {
        // The Montgomery reduction here is based on Algorithm 14.32 in
        // Handbook of Applied Cryptography
        // <http://cacr.uwaterloo.ca/hac/about/chap14.pdf>.

        let k = r0.wrapping_mul(INV);
        let (_, carry) = mac(r0, k, MODULUS.0[0], 0);
        let (r1, carry) = mac(r1, k, MODULUS.0[1], carry);
        let (r2, carry) = mac(r2, k, MODULUS.0[2], carry);
        let (r3, carry) = mac(r3, k, MODULUS.0[3], carry);
        let (r4, carry2) = adc(r4, 0, carry);

        let k = r1.wrapping_mul(INV);
        let (_, carry) = mac(r1, k, MODULUS.0[0], 0);
        let (r2, carry) = mac(r2, k, MODULUS.0[1], carry);
        let (r3, carry) = mac(r3, k, MODULUS.0[2], carry);
        let (r4, carry) = mac(r4, k, MODULUS.0[3], carry);
        let (r5, carry2) = adc(r5, carry2, carry);

        let k = r2.wrapping_mul(INV);
        let (_, carry) = mac(r2, k, MODULUS.0[0], 0);
        let (r3, carry) = mac(r3, k, MODULUS.0[1], carry);
        let (r4, carry) = mac(r4, k, MODULUS.0[2], carry);
        let (r5, carry) = mac(r5, k, MODULUS.0[3], carry);
        let (r6, carry2) = adc(r6, carry2, carry);

        let k = r3.wrapping_mul(INV);
        let (_, carry) = mac(r3, k, MODULUS.0[0], 0);
        let (r4, carry) = mac(r4, k, MODULUS.0[1], carry);
        let (r5, carry) = mac(r5, k, MODULUS.0[2], carry);
        let (r6, carry) = mac(r6, k, MODULUS.0[3], carry);
        let (r7, _) = adc(r7, carry2, carry);

        // Result may be within MODULUS of the correct value
        (&Scalar([r4, r5, r6, r7])).sub(&MODULUS)
    }

    /// Multiplies `rhs` by `self`, returning the result.
    #[inline]
    pub const fn mul(&self, rhs: &Self) -> Self {
        // Schoolbook multiplication

        let (r0, carry) = mac(0, self.0[0], rhs.0[0], 0);
        let (r1, carry) = mac(0, self.0[0], rhs.0[1], carry);
        let (r2, carry) = mac(0, self.0[0], rhs.0[2], carry);
        let (r3, r4) = mac(0, self.0[0], rhs.0[3], carry);

        let (r1, carry) = mac(r1, self.0[1], rhs.0[0], 0);
        let (r2, carry) = mac(r2, self.0[1], rhs.0[1], carry);
        let (r3, carry) = mac(r3, self.0[1], rhs.0[2], carry);
        let (r4, r5) = mac(r4, self.0[1], rhs.0[3], carry);

        let (r2, carry) = mac(r2, self.0[2], rhs.0[0], 0);
        let (r3, carry) = mac(r3, self.0[2], rhs.0[1], carry);
        let (r4, carry) = mac(r4, self.0[2], rhs.0[2], carry);
        let (r5, r6) = mac(r5, self.0[2], rhs.0[3], carry);

        let (r3, carry) = mac(r3, self.0[3], rhs.0[0], 0);
        let (r4, carry) = mac(r4, self.0[3], rhs.0[1], carry);
        let (r5, carry) = mac(r5, self.0[3], rhs.0[2], carry);
        let (r6, r7) = mac(r6, self.0[3], rhs.0[3], carry);

        Scalar::montgomery_reduce(r0, r1, r2, r3, r4, r5, r6, r7)
    }

    /// Subtracts `rhs` from `self`, returning the result.
    #[inline]
    pub const fn sub(&self, rhs: &Self) -> Self {
        let (d0, borrow) = sbb(self.0[0], rhs.0[0], 0);
        let (d1, borrow) = sbb(self.0[1], rhs.0[1], borrow);
        let (d2, borrow) = sbb(self.0[2], rhs.0[2], borrow);
        let (d3, borrow) = sbb(self.0[3], rhs.0[3], borrow);

        // If underflow occurred on the final limb, borrow = 0xfff...fff, otherwise
        // borrow = 0x000...000. Thus, we use it as a mask to conditionally add the modulus.
        let (d0, carry) = adc(d0, MODULUS.0[0] & borrow, 0);
        let (d1, carry) = adc(d1, MODULUS.0[1] & borrow, carry);
        let (d2, carry) = adc(d2, MODULUS.0[2] & borrow, carry);
        let (d3, _) = adc(d3, MODULUS.0[3] & borrow, carry);

        Scalar([d0, d1, d2, d3])
    }

    /// Adds `rhs` to `self`, returning the result.
    #[inline]
    pub const fn add(&self, rhs: &Self) -> Self {
        let (d0, carry) = adc(self.0[0], rhs.0[0], 0);
        let (d1, carry) = adc(self.0[1], rhs.0[1], carry);
        let (d2, carry) = adc(self.0[2], rhs.0[2], carry);
        let (d3, _) = adc(self.0[3], rhs.0[3], carry);

        // Attempt to subtract the modulus, to ensure the value
        // is smaller than the modulus.
        (&Scalar([d0, d1, d2, d3])).sub(&MODULUS)
    }

    /// Negates `self`.
    #[inline]
    pub const fn neg(&self) -> Self {
        // Subtract `self` from `MODULUS` to negate. Ignore the final
        // borrow because it cannot underflow; self is guaranteed to
        // be in the field.
        let (d0, borrow) = sbb(MODULUS.0[0], self.0[0], 0);
        let (d1, borrow) = sbb(MODULUS.0[1], self.0[1], borrow);
        let (d2, borrow) = sbb(MODULUS.0[2], self.0[2], borrow);
        let (d3, _) = sbb(MODULUS.0[3], self.0[3], borrow);

        // `tmp` could be `MODULUS` if `self` was zero. Create a mask that is
        // zero if `self` was zero, and `u64::max_value()` if self was nonzero.
        let mask = (((self.0[0] | self.0[1] | self.0[2] | self.0[3]) == 0) as u64).wrapping_sub(1);

        Scalar([d0 & mask, d1 & mask, d2 & mask, d3 & mask])
    }

    /// Hashes the input messages and domain separation tag to a `Scalar`
    #[cfg(feature = "hashing")]
    pub fn hash<X>(msg: &[u8], dst: &[u8]) -> Self
    where
        X: for<'a> elliptic_curve::hash2curve::ExpandMsg<'a>,
    {
        use elliptic_curve::hash2curve::Expander;

        let d = [dst];
        let mut expander = X::expand_message(&[msg], &d, 48).unwrap();
        let mut out = [0u8; 48];
        expander.fill_bytes(&mut out);
        Scalar::from_okm(&out)
    }
}

impl From<Scalar> for [u8; 32] {
    fn from(value: Scalar) -> [u8; 32] {
        value.to_le_bytes()
    }
}

impl<'a> From<&'a Scalar> for [u8; 32] {
    fn from(value: &'a Scalar) -> [u8; 32] {
        value.to_le_bytes()
    }
}

impl Field for Scalar {
    const ZERO: Self = Self::ZERO;
    const ONE: Self = Self::ONE;

    fn random(mut rng: impl RngCore) -> Self {
        let mut buf = [0; 64];
        rng.fill_bytes(&mut buf);
        Self::from_bytes_wide(&buf)
    }

    #[must_use]
    fn square(&self) -> Self {
        self.square()
    }

    #[must_use]
    fn double(&self) -> Self {
        self.double()
    }

    fn invert(&self) -> CtOption<Self> {
        self.invert()
    }

    fn sqrt_ratio(num: &Self, div: &Self) -> (Choice, Self) {
        ff::helpers::sqrt_ratio_generic(num, div)
    }

    fn sqrt(&self) -> CtOption<Self> {
        // (t - 1) // 2 = 6104339283789297388802252303364915521546564123189034618274734669823
        ff::helpers::sqrt_tonelli_shanks(
            self,
            [
                0x7fff_2dff_7fff_ffff,
                0x04d0_ec02_a9de_d201,
                0x94ce_bea4_199c_ec04,
                0x0000_0000_39f6_d3a9,
            ],
        )
    }

    fn is_zero_vartime(&self) -> bool {
        self.0 == Self::ZERO.0
    }
}

impl PrimeField for Scalar {
    type Repr = [u8; 32];

    fn from_repr(r: Self::Repr) -> CtOption<Self> {
        Self::from_le_bytes(&r)
    }

    fn to_repr(&self) -> Self::Repr {
        self.to_le_bytes()
    }

    fn is_odd(&self) -> Choice {
        Choice::from(self.to_le_bytes()[0] & 1)
    }

    const MODULUS: &'static str =
        "0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001";
    const NUM_BITS: u32 = MODULUS_BITS;
    const CAPACITY: u32 = Self::NUM_BITS - 1;
    const TWO_INV: Self = TWO_INV;
    const MULTIPLICATIVE_GENERATOR: Self = GENERATOR;
    const S: u32 = S;
    const ROOT_OF_UNITY: Self = ROOT_OF_UNITY;
    const ROOT_OF_UNITY_INV: Self = ROOT_OF_UNITY_INV;
    const DELTA: Self = DELTA;
}

impl AsRef<Scalar> for Scalar {
    fn as_ref(&self) -> &Scalar {
        self
    }
}

#[cfg(all(feature = "bits", not(target_pointer_width = "64")))]
type ReprBits = [u32; 8];

#[cfg(all(feature = "bits", target_pointer_width = "64"))]
type ReprBits = [u64; 4];

#[cfg(feature = "bits")]
impl PrimeFieldBits for Scalar {
    type ReprBits = ReprBits;

    fn to_le_bits(&self) -> FieldBits<Self::ReprBits> {
        let bytes = self.to_le_bytes();

        #[cfg(not(target_pointer_width = "64"))]
        let limbs = [
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
        ];

        #[cfg(target_pointer_width = "64")]
        let limbs = [
            u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        ];

        FieldBits::new(limbs)
    }

    fn char_le_bits() -> FieldBits<Self::ReprBits> {
        #[cfg(not(target_pointer_width = "64"))]
        {
            FieldBits::new(MODULUS_LIMBS_32)
        }

        #[cfg(target_pointer_width = "64")]
        FieldBits::new(MODULUS.0)
    }
}

impl<T> core::iter::Sum<T> for Scalar
where
    T: core::borrow::Borrow<Scalar>,
{
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = T>,
    {
        iter.fold(Self::ZERO, |acc, item| acc + item.borrow())
    }
}

impl<T> core::iter::Product<T> for Scalar
where
    T: core::borrow::Borrow<Scalar>,
{
    fn product<I>(iter: I) -> Self
    where
        I: Iterator<Item = T>,
    {
        iter.fold(Self::ONE, |acc, item| acc * item.borrow())
    }
}

impl fmt::LowerHex for Scalar {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let tmp = self.to_be_bytes();
        for &b in tmp.iter() {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

impl fmt::UpperHex for Scalar {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let tmp = self.to_be_bytes();
        for &b in tmp.iter() {
            write!(f, "{:02X}", b)?;
        }
        Ok(())
    }
}

/// A helper trait for serializing Scalars as little-endian instead
/// of big endian.
///
/// ```
/// use bls12_381_plus::{Scalar, ScalarLe};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// pub struct TestStruct {
///     #[serde(with = "ScalarLe")]
///     scalar: Scalar,
/// }
///
/// let s = Scalar::from_raw([3u64, 3u64, 3u64, 3u64]);
/// let t = TestStruct { scalar: s };
///
/// let ser1 = serde_json::to_string(&t).unwrap();
/// let ser2 = serde_json::to_string(&s).unwrap();
///
/// assert_eq!(ser1, "{\"scalar\":\"0300000000000000030000000000000003000000000000000300000000000000\"}");
/// assert_eq!(ser2, "\"0000000000000003000000000000000300000000000000030000000000000003\"");
/// ```
pub trait ScalarLe: Sized {
    /// Serialize scalar as little-endian
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;

    /// Deserialize into scalar from little-endian
    fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

impl ScalarLe for Scalar {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let bytes = self.to_le_bytes();
        if s.is_human_readable() {
            use serde::Serialize;

            let mut hexits = [0u8; 64];
            hex::encode_to_slice(bytes, &mut hexits).unwrap();
            let h = core::str::from_utf8(&hexits).unwrap();
            h.serialize(s)
        } else {
            use serde::ser::SerializeTuple;

            let mut tupler = s.serialize_tuple(bytes.len())?;
            for byte in bytes.iter() {
                tupler.serialize_element(byte)?;
            }
            tupler.end()
        }
    }

    fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::Deserialize;

        if d.is_human_readable() {
            let hex_str = <&str>::deserialize(d)?;
            let mut bytes = [0u8; 32];
            hex::decode_to_slice(hex_str, &mut bytes).unwrap();
            Option::<Scalar>::from(Self::from_le_bytes(&bytes))
                .ok_or_else(|| serde::de::Error::custom("invalid scalar"))
        } else {
            let bytes = <[u8; 32]>::deserialize(d)?;
            Option::<Scalar>::from(Self::from_le_bytes(&bytes))
                .ok_or_else(|| serde::de::Error::custom("invalid scalar"))
        }
    }
}

impl From<ScalarPrimitive<Bls12381G1>> for Scalar {
    fn from(value: ScalarPrimitive<Bls12381G1>) -> Self {
        Self::from_uint_unchecked(*value.as_uint())
    }
}

impl From<&ScalarPrimitive<Bls12381G1>> for Scalar {
    fn from(value: &ScalarPrimitive<Bls12381G1>) -> Self {
        Self::from_uint_unchecked(*value.as_uint())
    }
}

impl From<Scalar> for ScalarPrimitive<Bls12381G1> {
    fn from(value: Scalar) -> Self {
        ScalarPrimitive::from(&value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<&Scalar> for ScalarPrimitive<Bls12381G1> {
    fn from(value: &Scalar) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            let mut out = [0u64; 6];
            out[..4].copy_from_slice(&value.to_raw());
            ScalarPrimitive::new(U384::from_words(out)).unwrap()
        }
        #[cfg(target_pointer_width = "32")]
        {
            let mut out = [0u32; 12];
            raw_scalar_to_32bit_le_array(value, &mut out);
            ScalarPrimitive::new(U384::from_words(out)).unwrap()
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<&Scalar> for ScalarPrimitive<Bls12381G1> {
    fn from(value: &Scalar) -> Self {
        let mut out = [0u32; 12];
        let arr = value.to_raw();
        // convert from [u64;4] to [u32;8]
        for i in 0..4 {
            out[2 * i] = (arr[i] >> 32) as u32;
            out[2 * i + 1] = arr[i] as u32;
        }
        ScalarPrimitive::new(U384::from_words(out)).unwrap()
    }
}

impl From<GenericArray<u8, U48>> for Scalar {
    fn from(value: GenericArray<u8, U48>) -> Self {
        Self::from_uint_unchecked(U384::from_be_byte_array(value))
    }
}

impl From<Scalar> for GenericArray<u8, U48> {
    fn from(value: Scalar) -> Self {
        let mut arr = GenericArray::<u8, U48>::default();
        arr[16..].copy_from_slice(&value.to_be_bytes());
        arr
    }
}

impl From<GenericArray<u8, U32>> for Scalar {
    fn from(value: GenericArray<u8, U32>) -> Self {
        let arr: [u8; 32] = <[u8; 32]>::try_from(value.as_slice()).unwrap();
        Self::from_be_bytes(&arr).unwrap()
    }
}

impl From<Scalar> for GenericArray<u8, U32> {
    fn from(value: Scalar) -> Self {
        GenericArray::clone_from_slice(&value.to_be_bytes())
    }
}

impl From<U256> for Scalar {
    fn from(value: U256) -> Self {
        Self::reduce(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Scalar> for U256 {
    fn from(value: Scalar) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            let arr = value.to_raw();
            U256::from_words(arr)
        }
        #[cfg(target_pointer_width = "32")]
        {
            let mut out = [0u32; 8];
            raw_scalar_to_32bit_le_array(&value, &mut out);
            U256::from_words(out)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Scalar> for U256 {
    fn from(value: Scalar) -> Self {
        let arr = value.to_raw();
        // convert from [u64;4] to [u32;8]
        let mut arr32 = [0u32; 8];
        for i in 0..4 {
            arr32[2 * i] = (arr[i] >> 32) as u32;
            arr32[2 * i + 1] = arr[i] as u32;
        }
        arr32.reverse();
        U256::from_words(arr32)
    }
}

impl From<U384> for Scalar {
    fn from(value: U384) -> Self {
        Self::from_uint_unchecked(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Scalar> for U384 {
    fn from(value: Scalar) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            let raw = value.to_raw();
            let arr = [0u64, 0u64, raw[3], raw[2], raw[1], raw[0]];
            U384::from_words(arr)
        }
        #[cfg(target_pointer_width = "32")]
        {
            let mut arr = [0u32; 12];
            raw_scalar_to_32bit_le_array(&value, &mut arr);
            U384::from_words(arr)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Scalar> for U384 {
    fn from(value: Scalar) -> Self {
        let raw = value.to_raw();
        // convert from [u64;4] to [u32;12]
        let mut arr = [0u32; 12];
        for i in 0..4 {
            arr[2 * i] = (raw[i] >> 32) as u32;
            arr[2 * i + 1] = raw[i] as u32;
        }
        arr.reverse();
        U384::from_words(arr)
    }
}

impl From<U512> for Scalar {
    fn from(value: U512) -> Self {
        Self::reduce(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Scalar> for U512 {
    fn from(value: Scalar) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            let raw = value.to_raw();
            let arr = [0u64, 0u64, 0u64, 0u64, raw[3], raw[2], raw[1], raw[0]];
            U512::from_words(arr)
        }
        #[cfg(target_pointer_width = "32")]
        {
            let mut arr = [0u32; 16];
            raw_scalar_to_32bit_le_array(&value, &mut arr);
            U512::from_words(arr)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Scalar> for U512 {
    fn from(value: Scalar) -> Self {
        let raw = value.to_raw();
        // convert from [u64;4] to [u32;16]
        let mut arr = [0u32; 16];
        for i in 0..4 {
            arr[2 * i] = (raw[i] >> 32) as u32;
            arr[2 * i + 1] = raw[i] as u32;
        }
        arr.reverse();
        U512::from_words(arr)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl FromUintUnchecked for Scalar {
    type Uint = U384;

    fn from_uint_unchecked(uint: Self::Uint) -> Self {
        let mut out = [0u64; 4];
        #[cfg(target_pointer_width = "64")]
        {
            out.copy_from_slice(&uint.as_words()[..4]);
            Scalar::from_raw(out)
        }
        #[cfg(target_pointer_width = "32")]
        {
            let words = uint.as_words();
            let mut i = 0;
            for index in out.iter_mut() {
                *index = (words[i + 1] as u64) << 32;
                *index |= words[i] as u64;
                i += 2;
            }
            Scalar::from_raw(out)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl FromUintUnchecked for Scalar {
    type Uint = U384;

    fn from_uint_unchecked(uint: Self::Uint) -> Self {
        let mut out = [0u64; 4];
        let arr = uint.as_words();
        // convert from [u32;8] to [u64;4]
        for i in 0..4 {
            out[i] = (arr[2 * i] as u64) << 32 | arr[2 * i + 1] as u64;
        }
        out.reverse();
        Scalar::from_raw(out)
    }
}

impl Invert for Scalar {
    type Output = CtOption<Self>;

    fn invert(&self) -> Self::Output {
        self.invert()
    }
}

impl IsHigh for Scalar {
    fn is_high(&self) -> Choice {
        let mut borrow = 0;
        for i in 0..4 {
            let (_, b) = sbb(HALF_MODULUS.0[i], self.0[i], borrow);
            borrow = b;
        }
        ((borrow == u64::MAX) as u8).into()
    }
}

impl core::ops::Shr<usize> for Scalar {
    type Output = Self;

    fn shr(self, mut rhs: usize) -> Self::Output {
        // TODO: look for a more efficient method to do this
        let mut tmp = self;
        while rhs > 0 {
            tmp *= TWO_INV;
            rhs -= 1;
        }
        tmp
    }
}

impl core::ops::Shr<usize> for &Scalar {
    type Output = Scalar;

    fn shr(self, rhs: usize) -> Self::Output {
        *self >> rhs
    }
}

impl core::ops::ShrAssign<usize> for Scalar {
    fn shr_assign(&mut self, rhs: usize) {
        *self = *self >> rhs;
    }
}

impl Reduce<U256> for Scalar {
    type Bytes = GenericArray<u8, U32>;

    fn reduce(n: U256) -> Self {
        let mut out = [0u8; 48];
        out[..32].copy_from_slice(&n.to_be_bytes());
        Self::from_okm(&out)
    }

    fn reduce_bytes(bytes: &Self::Bytes) -> Self {
        Self::reduce(U256::from_be_byte_array(*bytes))
    }
}

impl Reduce<U384> for Scalar {
    type Bytes = GenericArray<u8, U48>;

    fn reduce(n: U384) -> Self {
        Self::from_okm(&n.to_be_bytes())
    }

    fn reduce_bytes(bytes: &Self::Bytes) -> Self {
        Self::reduce(U384::from_be_byte_array(*bytes))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Reduce<U512> for Scalar {
    type Bytes = GenericArray<u8, U64>;

    fn reduce(n: U512) -> Self {
        #[cfg(target_pointer_width = "64")]
        {
            Self::from_u512(*n.as_words())
        }
        #[cfg(target_pointer_width = "32")]
        {
            let words = n.as_words();
            let mut arr = [0u64; 8];
            let mut i = 0;
            for index in arr.iter_mut() {
                *index = (words[i + 1] as u64) << 32;
                *index |= words[i] as u64;
                i += 2;
            }
            Self::from_u512(arr)
        }
    }

    fn reduce_bytes(bytes: &Self::Bytes) -> Self {
        Self::reduce(U512::from_be_byte_array(*bytes))
    }
}

#[cfg(target_pointer_width = "32")]
fn raw_scalar_to_32bit_le_array(scalar: &Scalar, arr: &mut [u32]) {
    let raw = scalar.to_raw();
    let mut i = 0;
    let mut j = 0;

    while j < raw.len() {
        arr[i] = raw[j] as u32;
        arr[i + 1] = (raw[j] >> 32) as u32;

        i += 2;
        j += 1;
    }
}

#[test]
fn test_constants() {
    assert_eq!(
        Scalar::MODULUS,
        "0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001",
    );

    assert_eq!(Scalar::from(2u64) * Scalar::TWO_INV, Scalar::ONE);

    assert_eq!(
        Scalar::ROOT_OF_UNITY * Scalar::ROOT_OF_UNITY_INV,
        Scalar::ONE,
    );

    // ROOT_OF_UNITY^{2^s} mod m == 1
    assert_eq!(
        Scalar::ROOT_OF_UNITY.pow(&[1u64 << Scalar::S, 0, 0, 0]),
        Scalar::ONE,
    );

    // DELTA^{t} mod m == 1
    assert_eq!(
        Scalar::DELTA.pow(&[
            0xfffe_5bfe_ffff_ffff,
            0x09a1_d805_53bd_a402,
            0x299d_7d48_3339_d808,
            0x0000_0000_73ed_a753,
        ]),
        Scalar::ONE,
    );
}

#[test]
fn test_inv() {
    // Compute -(q^{-1} mod 2^64) mod 2^64 by exponentiating
    // by totient(2**64) - 1

    let mut inv = 1u64;
    for _ in 0..63 {
        inv = inv.wrapping_mul(inv);
        inv = inv.wrapping_mul(MODULUS.0[0]);
    }
    inv = inv.wrapping_neg();

    assert_eq!(inv, INV);
}

#[cfg(feature = "std")]
#[test]
fn test_debug() {
    assert_eq!(
        format!("{:?}", Scalar::ZERO),
        "0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert_eq!(
        format!("{:?}", Scalar::ONE),
        "0000000000000000000000000000000000000000000000000000000000000001"
    );
    assert_eq!(
        format!("{:?}", R2),
        "1824b159acc5056f998c4fefecbc4ff55884b7fa0003480200000001fffffffe"
    );
}

#[test]
fn test_equality() {
    assert_eq!(Scalar::ZERO, Scalar::ZERO);
    assert_eq!(Scalar::ONE, Scalar::ONE);
    assert_eq!(R2, R2);

    assert!(Scalar::ZERO != Scalar::ONE);
    assert!(Scalar::ONE != R2);
}

#[test]
fn test_to_bytes() {
    assert_eq!(
        Scalar::ZERO.to_le_bytes(),
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]
    );

    assert_eq!(
        Scalar::ONE.to_le_bytes(),
        [
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]
    );

    assert_eq!(
        R2.to_le_bytes(),
        [
            254, 255, 255, 255, 1, 0, 0, 0, 2, 72, 3, 0, 250, 183, 132, 88, 245, 79, 188, 236, 239,
            79, 140, 153, 111, 5, 197, 172, 89, 177, 36, 24
        ]
    );

    assert_eq!(
        (-&Scalar::ONE).to_le_bytes(),
        [
            0, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 115
        ]
    );
}

#[test]
fn test_from_bytes() {
    assert_eq!(
        Scalar::from_le_bytes(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ])
        .unwrap(),
        Scalar::ZERO
    );

    assert_eq!(
        Scalar::from_le_bytes(&[
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ])
        .unwrap(),
        Scalar::ONE
    );

    assert_eq!(
        Scalar::from_le_bytes(&[
            254, 255, 255, 255, 1, 0, 0, 0, 2, 72, 3, 0, 250, 183, 132, 88, 245, 79, 188, 236, 239,
            79, 140, 153, 111, 5, 197, 172, 89, 177, 36, 24
        ])
        .unwrap(),
        R2
    );

    // -1 should work
    assert!(bool::from(
        Scalar::from_le_bytes(&[
            0, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 115
        ])
        .is_some()
    ));

    // modulus is invalid
    assert!(bool::from(
        Scalar::from_le_bytes(&[
            1, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 115
        ])
        .is_none()
    ));

    // Anything larger than the modulus is invalid
    assert!(bool::from(
        Scalar::from_le_bytes(&[
            2, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 115
        ])
        .is_none()
    ));
    assert!(bool::from(
        Scalar::from_le_bytes(&[
            1, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 58, 51, 72, 125, 157, 41, 83, 167, 237, 115
        ])
        .is_none()
    ));
    assert!(bool::from(
        Scalar::from_le_bytes(&[
            1, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 116
        ])
        .is_none()
    ));
}

#[test]
fn test_from_u512_zero() {
    assert_eq!(
        Scalar::ZERO,
        Scalar::from_u512([
            MODULUS.0[0],
            MODULUS.0[1],
            MODULUS.0[2],
            MODULUS.0[3],
            0,
            0,
            0,
            0
        ])
    );
}

#[test]
fn test_from_u512_r() {
    assert_eq!(R, Scalar::from_u512([1, 0, 0, 0, 0, 0, 0, 0]));
}

#[test]
fn test_from_u512_r2() {
    assert_eq!(R2, Scalar::from_u512([0, 0, 0, 0, 1, 0, 0, 0]));
}

#[test]
fn test_from_u512_max() {
    let max_u64 = 0xffff_ffff_ffff_ffff;
    assert_eq!(
        R3 - R,
        Scalar::from_u512([max_u64, max_u64, max_u64, max_u64, max_u64, max_u64, max_u64, max_u64])
    );
}

#[test]
fn test_from_bytes_wide_r2() {
    assert_eq!(
        R2,
        Scalar::from_bytes_wide(&[
            254, 255, 255, 255, 1, 0, 0, 0, 2, 72, 3, 0, 250, 183, 132, 88, 245, 79, 188, 236, 239,
            79, 140, 153, 111, 5, 197, 172, 89, 177, 36, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])
    );
}

#[test]
fn test_from_bytes_wide_negative_one() {
    assert_eq!(
        -&Scalar::ONE,
        Scalar::from_bytes_wide(&[
            0, 0, 0, 0, 255, 255, 255, 255, 254, 91, 254, 255, 2, 164, 189, 83, 5, 216, 161, 9, 8,
            216, 57, 51, 72, 125, 157, 41, 83, 167, 237, 115, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])
    );
}

#[test]
fn test_from_bytes_wide_maximum() {
    assert_eq!(
        Scalar([
            0xc62c_1805_439b_73b1,
            0xc2b9_551e_8ced_218e,
            0xda44_ec81_daf9_a422,
            0x5605_aa60_1c16_2e79,
        ]),
        Scalar::from_bytes_wide(&[0xff; 64])
    );
}

#[test]
fn test_zero() {
    assert_eq!(Scalar::ZERO, -&Scalar::ZERO);
    assert_eq!(Scalar::ZERO, Scalar::ZERO + Scalar::ZERO);
    assert_eq!(Scalar::ZERO, Scalar::ZERO - Scalar::ZERO);
    assert_eq!(Scalar::ZERO, Scalar::ZERO * Scalar::ZERO);
}

#[cfg(test)]
const LARGEST: Scalar = Scalar([
    0xffff_ffff_0000_0000,
    0x53bd_a402_fffe_5bfe,
    0x3339_d808_09a1_d805,
    0x73ed_a753_299d_7d48,
]);

#[test]
fn test_addition() {
    let mut tmp = LARGEST;
    tmp += &LARGEST;

    assert_eq!(
        tmp,
        Scalar([
            0xffff_fffe_ffff_ffff,
            0x53bd_a402_fffe_5bfe,
            0x3339_d808_09a1_d805,
            0x73ed_a753_299d_7d48,
        ])
    );

    let mut tmp = LARGEST;
    tmp += &Scalar([1, 0, 0, 0]);

    assert_eq!(tmp, Scalar::ZERO);
}

#[test]
fn test_negation() {
    let tmp = -&LARGEST;

    assert_eq!(tmp, Scalar([1, 0, 0, 0]));

    let tmp = -&Scalar::ZERO;
    assert_eq!(tmp, Scalar::ZERO);
    let tmp = -&Scalar([1, 0, 0, 0]);
    assert_eq!(tmp, LARGEST);
}

#[test]
fn test_subtraction() {
    let mut tmp = LARGEST;
    tmp -= &LARGEST;

    assert_eq!(tmp, Scalar::ZERO);

    let mut tmp = Scalar::ZERO;
    tmp -= &LARGEST;

    let mut tmp2 = MODULUS;
    tmp2 -= &LARGEST;

    assert_eq!(tmp, tmp2);
}

#[test]
fn test_multiplication() {
    let mut cur = LARGEST;

    for _ in 0..100 {
        let mut tmp = cur;
        tmp *= &cur;

        let mut tmp2 = Scalar::ZERO;
        for b in cur
            .to_le_bytes()
            .iter()
            .rev()
            .flat_map(|byte| (0..8).rev().map(move |i| ((byte >> i) & 1u8) == 1u8))
        {
            let tmp3 = tmp2;
            tmp2.add_assign(&tmp3);

            if b {
                tmp2.add_assign(&cur);
            }
        }

        assert_eq!(tmp, tmp2);

        cur.add_assign(&LARGEST);
    }
}

#[test]
fn test_squaring() {
    let mut cur = LARGEST;

    for _ in 0..100 {
        let mut tmp = cur;
        tmp = tmp.square();

        let mut tmp2 = Scalar::ZERO;
        for b in cur
            .to_le_bytes()
            .iter()
            .rev()
            .flat_map(|byte| (0..8).rev().map(move |i| ((byte >> i) & 1u8) == 1u8))
        {
            let tmp3 = tmp2;
            tmp2.add_assign(&tmp3);

            if b {
                tmp2.add_assign(&cur);
            }
        }

        assert_eq!(tmp, tmp2);

        cur.add_assign(&LARGEST);
    }
}

#[test]
fn test_inversion() {
    assert!(bool::from(Scalar::ZERO.invert().is_none()));
    assert_eq!(Scalar::ONE.invert().unwrap(), Scalar::ONE);
    assert_eq!((-&Scalar::ONE).invert().unwrap(), -&Scalar::ONE);

    let mut tmp = R2;

    for _ in 0..100 {
        let mut tmp2 = tmp.invert().unwrap();
        tmp2.mul_assign(&tmp);

        assert_eq!(tmp2, Scalar::ONE);

        tmp.add_assign(&R2);
    }
}

#[test]
fn test_invert_is_pow() {
    let q_minus_2 = [
        0xffff_fffe_ffff_ffff,
        0x53bd_a402_fffe_5bfe,
        0x3339_d808_09a1_d805,
        0x73ed_a753_299d_7d48,
    ];

    let mut r1 = R;
    let mut r2 = R;
    let mut r3 = R;

    for _ in 0..100 {
        r1 = r1.invert().unwrap();
        r2 = r2.pow_vartime(&q_minus_2);
        r3 = r3.pow(&q_minus_2);

        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        // Add R so we check something different next time around
        r1.add_assign(&R);
        r2 = r1;
        r3 = r1;
    }
}

#[test]
fn test_sqrt() {
    {
        assert_eq!(Scalar::ZERO.sqrt().unwrap(), Scalar::ZERO);
    }

    let mut square = Scalar([
        0x46cd_85a5_f273_077e,
        0x1d30_c47d_d68f_c735,
        0x77f6_56f6_0bec_a0eb,
        0x494a_a01b_df32_468d,
    ]);

    let mut none_count = 0;

    for _ in 0..100 {
        let square_root = square.sqrt();
        if bool::from(square_root.is_none()) {
            none_count += 1;
        } else {
            assert_eq!(square_root.unwrap() * square_root.unwrap(), square);
        }
        square -= Scalar::ONE;
    }

    assert_eq!(49, none_count);
}

#[test]
fn test_from_raw() {
    assert_eq!(
        Scalar::from_raw([
            0x0001_ffff_fffd,
            0x5884_b7fa_0003_4802,
            0x998c_4fef_ecbc_4ff5,
            0x1824_b159_acc5_056f,
        ]),
        Scalar::from_raw([0xffff_ffff_ffff_ffff; 4])
    );

    assert_eq!(Scalar::from_raw(MODULUS.0), Scalar::ZERO);

    assert_eq!(Scalar::from_raw([1, 0, 0, 0]), R);
}

#[test]
fn test_double() {
    let a = Scalar::from_raw([
        0x1fff_3231_233f_fffd,
        0x4884_b7fa_0003_4802,
        0x998c_4fef_ecbc_4ff3,
        0x1824_b159_acc5_0562,
    ]);

    assert_eq!(a.double(), a + a);
}

#[test]
fn test_from_okm() {
    let okm = [
        155, 244, 205, 103, 163, 209, 47, 21, 160, 157, 37, 214, 5, 190, 2, 104, 223, 213, 41, 196,
        96, 200, 48, 201, 176, 145, 160, 209, 98, 168, 107, 154, 167, 197, 41, 218, 168, 132, 185,
        95, 111, 233, 85, 102, 45, 243, 24, 145,
    ];
    let expected = [
        184, 141, 14, 25, 196, 12, 5, 65, 222, 229, 103, 132, 86, 28, 224, 249, 100, 61, 100, 238,
        234, 250, 153, 140, 126, 148, 80, 19, 66, 92, 178, 14,
    ];
    let actual = Scalar::from_okm(&okm).to_le_bytes();
    assert_eq!(actual, expected)
}

#[test]
fn test_zeroize() {
    use zeroize::Zeroize;

    let mut a = Scalar::from_raw([
        0x1fff_3231_233f_fffd,
        0x4884_b7fa_0003_4802,
        0x998c_4fef_ecbc_4ff3,
        0x1824_b159_acc5_0562,
    ]);
    a.zeroize();
    assert!(bool::from(a.is_zero()));
}

#[test]
fn test_serialization() {
    let s1 = GENERATOR;

    let vec = serde_bare::to_vec(&s1).unwrap();
    let s2: Scalar = serde_bare::from_slice(&vec).unwrap();

    assert_eq!(s1, s2);

    let hex1 = serde_json::to_string(&s1).unwrap();
    let s2: Scalar = serde_json::from_str(&hex1).unwrap();
    assert_eq!(s1, s2);
}

#[cfg(feature = "alloc")]
#[test]
fn test_le_serialize() {
    {
        let mut st = alloc::vec::Vec::new();
        let mut writer = serde_json::Serializer::new(&mut st);
        ScalarLe::serialize(&Scalar::ONE, &mut writer).unwrap();
        assert_eq!(
            st.as_slice(),
            "\"0100000000000000000000000000000000000000000000000000000000000000\"".as_bytes()
        );
    }
    {
        let st = serde_json::to_string(&Scalar::ONE).unwrap();
        assert_eq!(
            st,
            "\"0000000000000000000000000000000000000000000000000000000000000001\""
        );
    }
}

#[test]
fn test_hex() {
    let s1 = R2;
    let hex = format!("{:x}", s1);
    let s2 = Scalar::from_be_hex(&hex);
    assert_eq!(s2.is_some().unwrap_u8(), 1u8);
    let s2 = s2.unwrap();
    assert_eq!(s1, s2);
    let hex = hex::encode(s1.to_le_bytes());
    let s2 = Scalar::from_le_hex(&hex);
    assert_eq!(s2.is_some().unwrap_u8(), 1u8);
    let s2 = s2.unwrap();
    assert_eq!(s1, s2);
}

#[test]
fn test_shr() {
    let two = Scalar::ONE + Scalar::ONE;
    assert_eq!(Scalar::ONE, two >> 1);

    assert_eq!(two, (two + two) >> 1);

    let ninety_six = Scalar::from(96u64);
    let forty_eight = Scalar::from(48u64);
    let res = ninety_six >> 1;
    assert_eq!(forty_eight, res);
}

#[test]
fn test_reduce() {
    let mut t = U384::from_be_hex("0000000000000000000000000000000073eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");
    t <<= 2;
    t = t.wrapping_add(&U384::ONE);
    let m = Scalar::reduce(t);
    assert_eq!(m, Scalar::ONE);

    let mut t = U512::from_be_hex("000000000000000000000000000000000000000000000000000000000000000073eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001");
    t = t.wrapping_mul(&(U512::ONE.wrapping_add(&U512::ONE).wrapping_add(&U512::ONE)));
    t = t.wrapping_add(&U512::ONE);
    t = t.wrapping_add(&U512::ONE);
    t = t.wrapping_add(&U512::ONE);

    let m = Scalar::reduce(t);
    assert_eq!(m, Scalar::ONE + Scalar::ONE + Scalar::ONE);
}
