use super::Error;
pub(crate) const WORK_BYTES: usize = 32 * 5;
// Duplicating the 255-element period removes the remainder in mul/div.
// 512 entries also cover every sum of two u8 values, so bounds-check removal
// does not depend on proving that valid logarithms never equal 255.
const fn field() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0; 512];
    let mut log = [0; 256];
    let mut x = 1u16;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 256 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    (exp, log)
}
const FIELD: ([u8; 512], [u8; 256]) = field();
fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        FIELD.0[FIELD.1[a as usize] as usize + FIELD.1[b as usize] as usize]
    }
}
fn div(a: u8, b: u8) -> u8 {
    if a == 0 {
        0
    } else {
        FIELD.0[FIELD.1[a as usize] as usize + 255 - FIELD.1[b as usize] as usize]
    }
}
fn eval(p: &[u8], x: u8) -> u8 {
    let mut y = 0;
    for &c in p.iter().rev() {
        y = mul(y, x) ^ c;
    }
    y
}
fn syndromes(data: &[u8], n: usize, s: &mut [u8]) -> bool {
    let mut nonzero = false;
    let mut i = 0;
    // Independent accumulators let the CPU overlap table-load dependencies.
    while i + 4 <= n {
        let mut v = [0u8; 4];
        for &b in data {
            for (j, value) in v.iter_mut().enumerate() {
                *value = if *value == 0 {
                    b
                } else {
                    FIELD.0[FIELD.1[*value as usize] as usize + i + j] ^ b
                };
            }
        }
        s[i..i + 4].copy_from_slice(&v);
        nonzero |= v.iter().any(|&v| v != 0);
        i += 4;
    }
    while i < n {
        let mut v = 0;
        for &b in data {
            v = if v == 0 {
                b
            } else {
                FIELD.0[FIELD.1[v as usize] as usize + i] ^ b
            };
        }
        s[i] = v;
        nonzero |= v != 0;
        i += 1;
    }
    nonzero
}
pub(crate) fn correct(
    data: &mut [u8],
    n: usize,
    reserve: usize,
    work: &mut [u8],
) -> Result<usize, Error> {
    let (s, rest) = work.split_at_mut(32);
    let (c, rest) = rest.split_at_mut(32);
    let (b, rest) = rest.split_at_mut(32);
    let (t, omega) = rest.split_at_mut(32);
    work_clear(s, c, b, t, omega);
    if !syndromes(data, n, s) {
        return Ok(0);
    }
    c[0] = 1;
    b[0] = 1;
    let mut degree = 0;
    let mut shift = 1;
    let mut previous = 1;
    for k in 0..n {
        let mut delta = s[k];
        for i in 1..=degree {
            delta ^= mul(c[i], s[k - i]);
        }
        if delta == 0 {
            shift += 1;
            continue;
        }
        let factor = div(delta, previous);
        t.copy_from_slice(c);
        for i in shift..32 {
            c[i] ^= mul(factor, b[i - shift]);
        }
        if 2 * degree <= k {
            degree = k + 1 - degree;
            b.copy_from_slice(t);
            previous = delta;
            shift = 1;
        } else {
            shift += 1;
        }
    }
    if degree == 0 || degree * 2 > n - reserve {
        return Err(Error::ReedSolomon);
    }
    for i in 0..n {
        for j in 0..=degree.min(i) {
            omega[i] ^= mul(c[j], s[i - j]);
        }
    }
    let mut roots = 0;
    for pos in 0..data.len() {
        let x = FIELD.0[pos];
        let inv = FIELD.0[(255 - pos) % 255];
        if eval(&c[..=degree], inv) != 0 {
            continue;
        }
        let mut deriv = 0;
        let mut power = 1;
        for (j, &coefficient) in c.iter().enumerate().take(degree + 1).skip(1) {
            if j % 2 == 1 {
                deriv ^= mul(coefficient, power);
            }
            power = mul(power, inv);
        }
        if deriv == 0 {
            return Err(Error::ReedSolomon);
        }
        let magnitude = div(mul(x, eval(&omega[..n], inv)), deriv);
        if magnitude == 0 {
            return Err(Error::ReedSolomon);
        }
        let index = data.len() - 1 - pos;
        data[index] ^= magnitude;
        roots += 1;
    }
    if roots != degree || syndromes(data, n, s) {
        Err(Error::ReedSolomon)
    } else {
        Ok(roots)
    }
}
fn work_clear(a: &mut [u8], b: &mut [u8], c: &mut [u8], d: &mut [u8], e: &mut [u8]) {
    a.fill(0);
    b.fill(0);
    c.fill(0);
    d.fill(0);
    e.fill(0);
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    // Independent polynomial-basis arithmetic, deliberately not log-table based.
    fn reference_mul(mut a: u8, mut b: u8) -> u8 {
        let mut result = 0;
        for _ in 0..8 {
            if b & 1 != 0 {
                result ^= a;
            }
            let carry = a & 0x80 != 0;
            a <<= 1;
            if carry {
                a ^= 0x1d;
            }
            b >>= 1;
        }
        result
    }
    fn reference_pow(mut a: u8, mut exponent: usize) -> u8 {
        let mut result = 1;
        while exponent != 0 {
            if exponent & 1 != 0 {
                result = reference_mul(result, a);
            }
            a = reference_mul(a, a);
            exponent >>= 1;
        }
        result
    }
    #[test]
    fn all_65536_products_and_quotients_match_independent_arithmetic() {
        let mut inverse = [0; 256];
        for (b, value) in inverse.iter_mut().enumerate().skip(1) {
            *value = reference_pow(b as u8, 254);
        }
        let mut maximum_mul_index = 0;
        let mut maximum_div_index = 0;
        for a in 0..=255u16 {
            for b in 0..=255u16 {
                let (a, b) = (a as u8, b as u8);
                assert_eq!(mul(a, b), reference_mul(a, b), "mul({a}, {b})");
                // Division by zero is not a GF operation. Preserve the existing
                // implementation's value (log[0] == 0), without endorsing its use.
                let expected = if b == 0 {
                    a
                } else {
                    reference_mul(a, inverse[b as usize])
                };
                assert_eq!(div(a, b), expected, "div({a}, {b})");
                if a != 0 && b != 0 {
                    maximum_mul_index = maximum_mul_index
                        .max(FIELD.1[a as usize] as usize + FIELD.1[b as usize] as usize);
                }
                if a != 0 {
                    maximum_div_index = maximum_div_index
                        .max(FIELD.1[a as usize] as usize + 255 - FIELD.1[b as usize] as usize);
                }
            }
        }
        assert_eq!(maximum_mul_index, 508);
        assert_eq!(maximum_div_index, 509);
        for i in 0..255 {
            assert_eq!(FIELD.0[i], reference_pow(2, i));
            assert_eq!(FIELD.0[i], FIELD.0[i + 255]);
        }
    }
    #[test]
    fn syndromes_match_polynomial_reference_for_all_supported_counts() {
        let mut state = 0x12345678u32;
        for len in [0, 1, 2, 16, 255] {
            let data: std::vec::Vec<u8> = (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            for n in 0..=32 {
                let mut actual = [0xa5; 32];
                let nonzero = syndromes(&data, n, &mut actual);
                let mut any = false;
                for (i, &actual_value) in actual.iter().enumerate().take(n) {
                    let x = reference_pow(2, i);
                    let expected = data
                        .iter()
                        .fold(0, |acc, &byte| reference_mul(acc, x) ^ byte);
                    assert_eq!(actual_value, expected, "len={len}, n={n}, i={i}");
                    any |= expected != 0;
                }
                assert_eq!(nonzero, any);
                assert!(actual[n..].iter().all(|&x| x == 0xa5));
            }
        }
    }
    // Systematic encoder independent of the production decoder implementation.
    fn encode(data: &[u8], ec: usize) -> std::vec::Vec<u8> {
        let mut generator = std::vec![1];
        for root in 0..ec {
            let mut next = std::vec![0; generator.len() + 1];
            for (i, &coefficient) in generator.iter().enumerate() {
                next[i] ^= coefficient;
                next[i + 1] ^= reference_mul(coefficient, reference_pow(2, root));
            }
            generator = next;
        }
        let mut division = data.to_vec();
        division.resize(data.len() + ec, 0);
        for i in 0..data.len() {
            let coefficient = division[i];
            for j in 1..=ec {
                division[i + j] ^= reference_mul(coefficient, generator[j]);
            }
        }
        let mut word = data.to_vec();
        word.extend_from_slice(&division[data.len()..]);
        word
    }
    #[test]
    fn independently_encoded_words_correct_through_capacity_for_ec2_to_30() {
        for ec in 2..=30 {
            for data_len in [1, 255 - ec] {
                for seed in [0, 37] {
                    let data: std::vec::Vec<_> =
                        (0..data_len).map(|i| (i * 59 + seed) as u8).collect();
                    let original = encode(&data, ec);
                    for errors in [0, 1, ec / 2] {
                        let mut damaged = original.clone();
                        let len = damaged.len();
                        for i in 0..errors {
                            damaged[(seed + i) % len] ^= ((i * 29 + seed) % 255 + 1) as u8;
                        }
                        let mut scratch = [0xa5; WORK_BYTES];
                        assert_eq!(
                            correct(&mut damaged, ec, 0, &mut scratch),
                            Ok(errors),
                            "ec={ec}, data_len={data_len}, seed={seed}, errors={errors}"
                        );
                        assert_eq!(damaged, original);
                    }
                    // A reserve leaves less correction capacity; max errors must
                    // reject before applying a correction beyond that contract.
                    let mut damaged = original.clone();
                    for value in damaged.iter_mut().take(ec / 2) {
                        *value ^= 0x83;
                    }
                    assert_eq!(
                        correct(&mut damaged, ec, 2, &mut [0; WORK_BYTES]),
                        Err(Error::ReedSolomon)
                    );
                }
            }
        }
    }
}
