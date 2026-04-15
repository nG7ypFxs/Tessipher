use crate::Error;
pub(crate) const WORK_BYTES: usize = 32 * 5;
const fn field() -> ([u8; 255], [u8; 256]) {
    let mut exp = [0; 255];
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
    (exp, log)
}
const FIELD: ([u8; 255], [u8; 256]) = field();
fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        FIELD.0[(FIELD.1[a as usize] as usize + FIELD.1[b as usize] as usize) % 255]
    }
}
fn div(a: u8, b: u8) -> u8 {
    if a == 0 {
        0
    } else {
        FIELD.0[(FIELD.1[a as usize] as usize + 255 - FIELD.1[b as usize] as usize) % 255]
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
    for (i, syndrome) in s.iter_mut().enumerate().take(n) {
        let mut v = 0;
        for &b in data {
            v = mul(v, FIELD.0[i]) ^ b;
        }
        *syndrome = v;
        nonzero |= v != 0;
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
