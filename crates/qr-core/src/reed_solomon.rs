//! Reed-Solomon encoding and error correction over GF(256).
//!
//! Codewords are treated as coefficients of a polynomial with the first
//! codeword as the highest-degree term. The generator polynomial has
//! consecutive roots `α^0 .. α^(ec_len-1)`, matching the QR convention.

use alloc::vec;
use alloc::vec::Vec;

use crate::Error;
use crate::gf;

/// Builds the Reed-Solomon generator polynomial of degree `ec_len`.
///
/// Returned coefficients are in descending degree order with a leading `1`
/// (length `ec_len + 1`).
pub fn generator_poly(ec_len: usize) -> Vec<u8> {
    let mut g: Vec<u8> = vec![1];
    for i in 0..ec_len {
        let root = gf::exp(i as u16);
        let mut next = vec![0u8; g.len() + 1];
        for j in 0..g.len() {
            next[j] ^= g[j];
            next[j + 1] ^= gf::mul(g[j], root);
        }
        g = next;
    }
    g
}

/// Computes the `ec_len` error-correction codewords for `data`.
///
/// The returned vector has length `ec_len`.
pub fn rs_encode(data: &[u8], ec_len: usize) -> Vec<u8> {
    if ec_len == 0 {
        return Vec::new();
    }
    let generator = generator_poly(ec_len);
    let mut res = vec![0u8; data.len() + ec_len];
    res[..data.len()].copy_from_slice(data);
    for i in 0..data.len() {
        let coef = res[i];
        if coef != 0 {
            for j in 0..generator.len() {
                res[i + j] ^= gf::mul(generator[j], coef);
            }
        }
    }
    res[data.len()..].to_vec()
}

/// Evaluates `poly` (ascending: `poly[0]` is the constant term) at `x`.
fn eval(poly: &[u8], x: u8) -> u8 {
    let mut y = 0u8;
    for &c in poly.iter().rev() {
        y = gf::mul(y, x) ^ c;
    }
    y
}

/// Formal derivative of `poly` (ascending order) over GF(2): terms of even
/// degree vanish.
fn derivative(poly: &[u8]) -> Vec<u8> {
    if poly.len() <= 1 {
        return Vec::new();
    }
    let mut d = vec![0u8; poly.len() - 1];
    for j in 1..poly.len() {
        if j & 1 == 1 {
            d[j - 1] = poly[j];
        }
    }
    d
}

/// Berlekamp-Massey: returns the error-locator polynomial `Λ(x)` in ascending
/// order (`Λ[0] == 1`); its degree is the number of errors.
fn berlekamp_massey(syndromes: &[u8]) -> Vec<u8> {
    let mut cx: Vec<u8> = vec![1];
    let mut bx: Vec<u8> = vec![1];
    let mut l = 0usize;
    let mut m = 1usize;
    let mut b = 1u8;

    for i in 0..syndromes.len() {
        // Discrepancy.
        let mut delta = syndromes[i];
        for j in 1..=l {
            if j < cx.len() {
                delta ^= gf::mul(cx[j], syndromes[i - j]);
            }
        }

        if delta == 0 {
            m += 1;
        } else if 2 * l <= i {
            let t = cx.clone();
            let coef = gf::div(delta, b);
            if cx.len() < bx.len() + m {
                cx.resize(bx.len() + m, 0);
            }
            for j in 0..bx.len() {
                cx[j + m] ^= gf::mul(coef, bx[j]);
            }
            l = i + 1 - l;
            bx = t;
            b = delta;
            m = 1;
        } else {
            let coef = gf::div(delta, b);
            if cx.len() < bx.len() + m {
                cx.resize(bx.len() + m, 0);
            }
            for j in 0..bx.len() {
                cx[j + m] ^= gf::mul(coef, bx[j]);
            }
            m += 1;
        }
    }
    cx
}

/// Corrects errors in `codewords` in place (data followed by `ec_len` EC
/// codewords).
///
/// Returns the number of codewords that were corrected, or [`Error`] when the
/// error count exceeds the correction capacity (`ec_len / 2`) or the input is
/// malformed. On error the buffer may be partially modified; callers must
/// treat a returned `Err` as an unrecoverable block and discard the buffer.
pub fn rs_correct(codewords: &mut [u8], ec_len: usize) -> Result<usize, Error> {
    let n = codewords.len();
    if ec_len == 0 || ec_len >= n {
        return Err(Error::InvalidLength);
    }

    // Syndromes S_i = R(α^i) for i in 0..ec_len.
    let mut syndromes = vec![0u8; ec_len];
    let mut nonzero = false;
    for (i, syn) in syndromes.iter_mut().enumerate() {
        let x = gf::exp(i as u16);
        let mut s = 0u8;
        for &c in codewords.iter() {
            s = gf::mul(s, x) ^ c;
        }
        *syn = s;
        if s != 0 {
            nonzero = true;
        }
    }
    if !nonzero {
        return Ok(0);
    }

    let lambda = berlekamp_massey(&syndromes);
    let err_count = lambda.len().saturating_sub(1);
    if err_count == 0 || err_count > ec_len / 2 {
        return Err(Error::TooManyErrors);
    }

    // Chien search: codeword index i has locator X = α^(n-1-i); it is an error
    // location iff Λ(X^-1) == 0.
    let mut positions: Vec<usize> = Vec::new();
    let mut locators: Vec<u8> = Vec::new();
    for i in 0..n {
        let x = gf::exp(((n - 1 - i) % 255) as u16);
        if eval(&lambda, gf::inv(x)) == 0 {
            positions.push(i);
            locators.push(x);
        }
    }
    if positions.len() != err_count {
        return Err(Error::TooManyErrors);
    }

    // Error evaluator Ω(x) = S(x)·Λ(x) mod x^ec_len (both ascending).
    let mut omega = vec![0u8; ec_len];
    for (i, slot) in omega.iter_mut().enumerate() {
        let mut s = 0u8;
        for j in 0..=i {
            if j < lambda.len() {
                s ^= gf::mul(syndromes[i - j], lambda[j]);
            }
        }
        *slot = s;
    }
    let dlambda = derivative(&lambda);

    // Forney (first-consecutive-root exponent = 0): e = X · Ω(X^-1) / Λ'(X^-1).
    for (idx, &pos) in positions.iter().enumerate() {
        let x = locators[idx];
        let xinv = gf::inv(x);
        let num = eval(&omega, xinv);
        let den = eval(&dlambda, xinv);
        if den == 0 {
            return Err(Error::TooManyErrors);
        }
        let magnitude = gf::mul(x, gf::div(num, den));
        codewords[pos] ^= magnitude;
    }

    // Verify the corrected codeword has zero syndromes.
    for i in 0..ec_len {
        let x = gf::exp(i as u16);
        let mut s = 0u8;
        for &c in codewords.iter() {
            s = gf::mul(s, x) ^ c;
        }
        if s != 0 {
            return Err(Error::TooManyErrors);
        }
    }

    Ok(positions.len())
}
