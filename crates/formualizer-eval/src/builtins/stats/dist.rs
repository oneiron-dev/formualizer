//! Excel's probability distributions: the numerics and argument rules shared
//! by the current names (NORM.DIST, CHISQ.DIST.RT, ...) and the
//! compatibility names Excel still accepts and writes without an `_xlfn.`
//! prefix (NORMDIST, CHIDIST, ...), which are defined here.
//!
//! Excel for Windows 16.0.20430 settled the rules
//! (ops/excel-legacy-functions-probe-20261006.md):
//! - degrees of freedom are truncated to whole numbers (T, chi-square, F);
//! - a cumulative argument is a logical: a number (non-zero is TRUE), a
//!   logical, a blank, or the text "TRUE"/"FALSE" in any case; other text,
//!   numeric text included, is `#VALUE!`;
//! - an omitted bound of BETA.DIST or BETA.INV (`BETADIST(x,2,3,0,)`) takes
//!   its default;
//! - densities at the edge of the support: GAMMA.DIST at 0 is `#NUM!` for
//!   alpha <= 1 (0 above), BETA.DIST at A is `#NUM!` for alpha <= 1 (and at B
//!   for beta <= 1), WEIBULL.DIST at 0 is 0 for every alpha, CHISQ.DIST at 0
//!   is `#NUM!` for 1 degree of freedom and 1/2 for 2, F.DIST at 0 is `#NUM!`
//!   for 1 numerator degree of freedom and 1 for 2;
//! - a lower inverse takes probability 0 (CHISQ.INV(0,10) and F.INV are 0), a
//!   right-tailed one probability 1 (CHIINV(1,10) is 0), GAMMA.INV and
//!   BETA.INV take neither end, and T.INV.2T/TINV take 0 < p < 2
//!   (TINV(1.1,10) is T.INV(0.45,10));
//! - small probabilities are computed as themselves, never as 1 minus a value
//!   near 1: CHIDIST(1000,2) is 7.1E-218 and NORMSDIST(-37.5) is 4.6E-308.
//!
//! - the T and F functions and the chi-square distribution refuse degrees of
//!   freedom above 1E10; the chi-square inverses take them (CHIINV(0.5,1E11)
//!   is a number, though Excel's own is 2.7E-4 off the quantile there).
//!
//! The compatibility names keep their own signatures: NORMSDIST(z) and
//! LOGNORMDIST(x,mean,sd) are cumulative, CHIDIST, FDIST, CHIINV and FINV are
//! right-tailed, TDIST(x,df,tails) takes 1 or 2 tails (truncated) and x >= 0,
//! and HYPGEOMDIST and NEGBINOMDIST are probabilities without a cumulative
//! argument.

use super::{ln_gamma, scalar_like_value, std_norm_cdf, std_norm_inv, std_norm_pdf};
use crate::args::ArgSchema;
use crate::builtins::utils::coerce_num;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;
use std::f64::consts::PI;

/* ─────────────────────────── numerics ──────────────────────────── */

const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// Iteration cap of the series and continued fractions below. Near the centre
/// of a distribution of shape `a` they need about `sqrt(a)` terms, so this
/// covers shapes up to about 1E11 (CHIINV(0.5,1E10) is a shape of 5E9).
const MAX_TERMS: usize = 4_000_000;

/// Stirling's remainder `lnΓ(a) - ((a - 1/2) ln a - a + ln √(2π))`, for a >= 10.
fn stirling_rest(a: f64) -> f64 {
    let r = 1.0 / a;
    let r2 = r * r;
    r * (1.0 / 12.0
        + r2 * (-1.0 / 360.0
            + r2 * (1.0 / 1260.0
                + r2 * (-1.0 / 1680.0
                    + r2 * (1.0 / 1188.0 + r2 * (-691.0 / 360_360.0 + r2 / 156.0))))))
}

/// `ln(1 + d) - d`, without the cancellation of the two for small `d`.
fn log1pmx(d: f64) -> f64 {
    if d.abs() > 0.5 {
        return d.ln_1p() - d;
    }
    let mut power = -d * d;
    let mut sum = 0.0;
    let mut k = 2.0;
    for _ in 0..200 {
        let term = power / k;
        sum += term;
        if term.abs() <= f64::EPSILON * sum.abs() {
            break;
        }
        power *= -d;
        k += 1.0;
    }
    sum
}

/// `ln B(a, b)`. For a large argument the large lnΓ values cancel
/// analytically (Stirling) instead of numerically, so `ln B(0.5, 5E9)` keeps
/// its precision.
pub(super) fn ln_beta(a: f64, b: f64) -> f64 {
    let (small, large) = if a < b { (a, b) } else { (b, a) };
    let c = a + b;
    if small >= 10.0 {
        small * (small / c).ln()
            + large * (-small / c).ln_1p()
            + LN_SQRT_2PI
            + 0.5 * ((c / large).ln() - small.ln())
            + stirling_rest(small)
            + stirling_rest(large)
            - stirling_rest(c)
    } else if large >= 10.0 {
        ln_gamma(small) - small * large.ln() - (c - 0.5) * (small / large).ln_1p()
            + small
            + stirling_rest(large)
            - stirling_rest(c)
    } else {
        ln_gamma(a) + ln_gamma(b) - ln_gamma(c)
    }
}

/// `ln C(n, k)` for whole `0 <= k <= n`.
fn ln_choose(n: f64, k: f64) -> f64 {
    if k == 0.0 || k == n {
        0.0
    } else {
        -(n + 1.0).ln() - ln_beta(k + 1.0, n - k + 1.0)
    }
}

/// `x^a e^-x / Γ(a)`.
fn gamma_prefix(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if a < 10.0 {
        (a * x.ln() - x - ln_gamma(a)).exp()
    } else {
        (a * log1pmx((x - a) / a) - stirling_rest(a)).exp() * (a / (2.0 * PI)).sqrt()
    }
}

/// The regularized incomplete gamma functions `(P(a, x), Q(a, x))`; the
/// smaller of the two keeps its relative precision.
pub(super) fn gamma_inc(a: f64, x: f64) -> (f64, f64) {
    if x.is_nan() || a.is_nan() {
        return (f64::NAN, f64::NAN);
    }
    if x <= 0.0 {
        return (0.0, 1.0);
    }
    if x.is_infinite() {
        return (1.0, 0.0);
    }
    let prefix = gamma_prefix(a, x);
    if x < a + 1.0 {
        // P = prefix / a * (1 + x/(a+1) + x^2/((a+1)(a+2)) + ...)
        let (mut term, mut sum, mut n) = (1.0, 1.0, a);
        for _ in 0..MAX_TERMS {
            n += 1.0;
            term *= x / n;
            sum += term;
            if term <= sum * f64::EPSILON {
                break;
            }
        }
        let p = (prefix / a * sum).min(1.0);
        (p, 1.0 - p)
    } else {
        // Q = prefix * Legendre's continued fraction (modified Lentz).
        const TINY: f64 = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..MAX_TERMS {
            let i = i as f64;
            let an = -i * (i - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() <= 2.0 * f64::EPSILON {
                break;
            }
        }
        let q = (prefix * h).min(1.0);
        (1.0 - q, q)
    }
}

/// The density of the standard gamma distribution of shape `a` at `x > 0`.
fn gamma_density(a: f64, x: f64) -> f64 {
    gamma_prefix(a, x) / x
}

/// `x^a y^b / B(a, b)` with `y = 1 - x`, both given so that the one near 0
/// keeps its precision.
fn beta_prefix(a: f64, b: f64, x: f64, y: f64) -> f64 {
    if x <= 0.0 || y <= 0.0 {
        return 0.0;
    }
    let c = a + b;
    if a.min(b) >= 10.0 {
        // Around the centre x0 = a / c: x = x0 (1 + u), y = y0 (1 + v).
        let u = (x * c - a) / a;
        let v = (y * c - b) / b;
        (a * log1pmx(u) + b * log1pmx(v) - stirling_rest(a) - stirling_rest(b) + stirling_rest(c))
            .exp()
            * (a / (2.0 * PI) * (b / c)).sqrt()
    } else {
        let ln_x = if x <= 0.5 { x.ln() } else { (-y).ln_1p() };
        let ln_y = if y <= 0.5 { y.ln() } else { (-x).ln_1p() };
        (a * ln_x + b * ln_y - ln_beta(a, b)).exp()
    }
}

/// The continued fraction of `I_x(a, b)` (modified Lentz).
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..MAX_TERMS {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() <= 2.0 * f64::EPSILON {
            break;
        }
    }
    h
}

/// The regularized incomplete beta function and its complement,
/// `(I_x(a, b), 1 - I_x(a, b))`, with `y = 1 - x`; the smaller of the two
/// keeps its relative precision.
pub(super) fn beta_inc(a: f64, b: f64, x: f64, y: f64) -> (f64, f64) {
    if x.is_nan() || y.is_nan() {
        return (f64::NAN, f64::NAN);
    }
    if x <= 0.0 {
        return (0.0, 1.0);
    }
    if y <= 0.0 {
        return (1.0, 0.0);
    }
    let prefix = beta_prefix(a, b, x, y);
    if x < (a + 1.0) / (a + b + 2.0) {
        let i = (prefix * beta_cf(a, b, x) / a).min(1.0);
        (i, 1.0 - i)
    } else {
        let j = (prefix * beta_cf(b, a, y) / b).min(1.0);
        (1.0 - j, j)
    }
}

/// The density of the standard beta distribution at `x` (`y = 1 - x`), inside (0, 1).
fn beta_density(a: f64, b: f64, x: f64, y: f64) -> f64 {
    beta_prefix(a, b, x, y) / (x * y)
}

/// `P(T > t)` for Student's t with `df` degrees of freedom. Like Excel, a
/// `t` too small to change `df + t^2` is a tail of exactly 1/2
/// (TDIST(1E-10,5,2) is 1).
pub(super) fn t_upper(t: f64, df: f64) -> f64 {
    if t.is_nan() {
        return f64::NAN;
    }
    let t2 = t * t;
    let s = df + t2;
    let tail = if t2.is_infinite() {
        0.0
    } else if s == df {
        0.5
    } else {
        0.5 * beta_inc(df / 2.0, 0.5, df / s, t2 / s).0
    };
    if t >= 0.0 { tail } else { 1.0 - tail }
}

/// `P(T <= t)`.
pub(super) fn t_lower(t: f64, df: f64) -> f64 {
    if t < 0.0 {
        t_upper(-t, df)
    } else {
        1.0 - t_upper(t, df)
    }
}

fn t_density(t: f64, df: f64) -> f64 {
    (-(df + 1.0) / 2.0 * (t * t / df).ln_1p() - 0.5 * df.ln() - ln_beta(df / 2.0, 0.5)).exp()
}

/// `(P(F <= f), P(F > f))` for the F distribution.
pub(super) fn f_tails(f: f64, d1: f64, d2: f64) -> (f64, f64) {
    if f <= 0.0 {
        return (0.0, 1.0);
    }
    let p = d1 * f;
    if p.is_infinite() {
        return (1.0, 0.0);
    }
    let s = p + d2;
    beta_inc(d1 / 2.0, d2 / 2.0, p / s, d2 / s)
}

/// The F density at `f > 0`.
fn f_density(f: f64, d1: f64, d2: f64) -> f64 {
    let p = d1 * f;
    let s = p + d2;
    beta_prefix(d1 / 2.0, d2 / 2.0, p / s, d2 / s) / f
}

/// `(P(X <= x), P(X > x))` for chi-square with `df` degrees of freedom.
pub(super) fn chisq_tails(x: f64, df: f64) -> (f64, f64) {
    gamma_inc(df / 2.0, x / 2.0)
}

/// The chi-square density at `x > 0`.
fn chisq_density(x: f64, df: f64) -> f64 {
    gamma_prefix(df / 2.0, x / 2.0) / x
}

/// The root in `(lo, hi)` of an increasing function returning
/// `(value, slope)`: Newton steps kept inside a shrinking bracket, halving
/// (geometrically across a wide positive bracket) when a step leaves it.
fn solve_increasing(lo: f64, hi: f64, start: f64, f: impl Fn(f64) -> (f64, f64)) -> f64 {
    let (mut lo, mut hi) = (lo, hi);
    let mut x = start;
    for _ in 0..2000 {
        let (value, slope) = f(x);
        if value == 0.0 {
            return x;
        }
        if value < 0.0 {
            lo = x;
        } else {
            hi = x;
        }
        let newton = x - value / slope;
        let next = if newton.is_finite() && newton > lo && newton < hi {
            newton
        } else if hi.is_infinite() {
            2.0 * x.max(1.0)
        } else if lo <= 0.0 {
            hi / 16.0
        } else if hi > 8.0 * lo {
            (lo * hi).sqrt()
        } else {
            0.5 * (lo + hi)
        };
        if (next - x).abs() <= 2.0 * f64::EPSILON * next.abs()
            || (hi.is_finite() && hi - lo <= 2.0 * f64::EPSILON * hi.abs())
        {
            return next;
        }
        x = next;
    }
    x
}

/// The `z` with `P(a, z) = lower` and `Q(a, z) = upper` (`upper = 1 - lower`).
fn gamma_quantile(a: f64, lower: f64, upper: f64) -> f64 {
    if lower <= 0.0 {
        return 0.0;
    }
    if upper <= 0.0 {
        return f64::INFINITY;
    }
    let use_lower = lower <= upper;
    // Wilson–Hilferty, or the small-z power law when that is not positive.
    let n = if use_lower {
        std_norm_inv(lower)
    } else {
        std_norm_inv(upper).map(|z| -z)
    }
    .unwrap_or(0.0);
    let cube = 1.0 - 1.0 / (9.0 * a) + n / (3.0 * a.sqrt());
    let start = if cube > 0.0 {
        a * cube * cube * cube
    } else {
        ((lower.ln() + ln_gamma(a + 1.0)) / a).exp()
    };
    let start = if start > 0.0 && start.is_finite() {
        start
    } else {
        a
    };
    solve_increasing(0.0, f64::INFINITY, start, |z| {
        let (p, q) = gamma_inc(a, z);
        let slope = gamma_density(a, z);
        if use_lower {
            (p - lower, slope)
        } else {
            (upper - q, slope)
        }
    })
}

/// The `x` in (0, 1/2] with `I_x(a, b) = lower` (`1 - I_x(a, b) = upper`).
fn beta_quantile_low(a: f64, b: f64, lower: f64, upper: f64) -> f64 {
    let use_lower = lower <= upper;
    let start = ((lower.ln() + a.ln() + ln_beta(a, b)) / a)
        .exp()
        .clamp(f64::MIN_POSITIVE, 0.25);
    solve_increasing(0.0, 0.5, start, |x| {
        let y = 1.0 - x;
        let (i, j) = beta_inc(a, b, x, y);
        let slope = beta_density(a, b, x, y);
        if use_lower {
            (i - lower, slope)
        } else {
            (upper - j, slope)
        }
    })
}

/// `(x, 1 - x)` with `I_x(a, b) = lower` (`1 - I_x(a, b) = upper`); the one
/// of x and 1 - x that lies below 1/2 is solved for, so both keep their
/// precision.
fn beta_quantile(a: f64, b: f64, lower: f64, upper: f64) -> (f64, f64) {
    if lower <= 0.0 {
        return (0.0, 1.0);
    }
    if upper <= 0.0 {
        return (1.0, 0.0);
    }
    let (at_half, _) = beta_inc(a, b, 0.5, 0.5);
    if lower == at_half {
        (0.5, 0.5)
    } else if lower < at_half {
        let x = beta_quantile_low(a, b, lower, upper);
        (x, 1.0 - x)
    } else {
        let y = beta_quantile_low(b, a, upper, lower);
        (1.0 - y, y)
    }
}

/// The `t >= 0` with `P(T > t) = upper`, for `upper <= 1/2`.
fn t_upper_quantile(upper: f64, df: f64) -> f64 {
    if upper >= 0.5 {
        return 0.0;
    }
    // P(T > t) = I_x(df/2, 1/2) / 2 with x = df / (df + t^2).
    let (x, y) = beta_quantile(df / 2.0, 0.5, 2.0 * upper, 1.0 - 2.0 * upper);
    (df * y / x).sqrt()
}

/// The `f` with `P(F <= f) = lower` (`P(F > f) = upper`).
fn f_quantile(d1: f64, d2: f64, lower: f64, upper: f64) -> f64 {
    let (x, y) = beta_quantile(d1 / 2.0, d2 / 2.0, lower, upper);
    d2 * x / (d1 * y)
}

/// `P(X = k)` for the binomial distribution, whole `0 <= k <= n`.
pub(super) fn binom_pmf(k: f64, n: f64, p: f64) -> f64 {
    let q = 1.0 - p;
    if p == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if q == 0.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    if k == 0.0 {
        return (n * (-p).ln_1p()).exp();
    }
    if k == n {
        return (n * p.ln()).exp();
    }
    // C(n, k) = n / (k (n - k) B(k, n - k)).
    beta_prefix(k, n - k, p, q) * n / (k * (n - k))
}

/// `P(X <= k)` for the binomial distribution.
fn binom_cdf(k: f64, n: f64, p: f64) -> f64 {
    if k >= n || p == 0.0 {
        return 1.0;
    }
    if p == 1.0 {
        return 0.0;
    }
    beta_inc(n - k, k + 1.0, 1.0 - p, p).0
}

fn poisson_pmf(k: f64, mean: f64) -> f64 {
    if mean == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if k == 0.0 {
        return (-mean).exp();
    }
    gamma_prefix(k, mean) / k
}

fn hypgeom_pmf(s: f64, n: f64, m: f64, total: f64) -> f64 {
    (ln_choose(m, s) + ln_choose(total - m, n - s) - ln_choose(total, n)).exp()
}

/* ─────────────────────────── Excel's rules ──────────────────────────── */

/// `#NUM!` unless the arguments are in the function's domain.
fn domain(ok: bool) -> Result<(), ExcelError> {
    if ok {
        Ok(())
    } else {
        Err(ExcelError::new_num())
    }
}

/// The T and F functions and the chi-square distribution (not its inverses:
/// CHIINV(0.5,1E11) is a number) refuse degrees of freedom above 1E10.
const DF_MAX: f64 = 1e10;

/// Whole degrees of freedom in [1, 1E10].
fn df_ok(df: f64) -> bool {
    (1.0..=DF_MAX).contains(&df)
}

pub(super) fn norm_dist(x: f64, mean: f64, sd: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(sd > 0.0)?;
    let z = (x - mean) / sd;
    Ok(if cumulative {
        std_norm_cdf(z)
    } else {
        std_norm_pdf(z) / sd
    })
}

pub(super) fn norm_inv(p: f64, mean: f64, sd: f64) -> Result<f64, ExcelError> {
    domain(sd > 0.0)?;
    let z = std_norm_inv(p).ok_or_else(ExcelError::new_num)?;
    Ok(mean + z * sd)
}

pub(super) fn lognorm_dist(
    x: f64,
    mean: f64,
    sd: f64,
    cumulative: bool,
) -> Result<f64, ExcelError> {
    domain(x > 0.0 && sd > 0.0)?;
    let z = (x.ln() - mean) / sd;
    Ok(if cumulative {
        std_norm_cdf(z)
    } else {
        std_norm_pdf(z) / (x * sd)
    })
}

pub(super) fn lognorm_inv(p: f64, mean: f64, sd: f64) -> Result<f64, ExcelError> {
    Ok(norm_inv(p, mean, sd)?.exp())
}

pub(super) fn t_dist(x: f64, df: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(df_ok(df))?;
    Ok(if cumulative {
        t_lower(x, df)
    } else {
        t_density(x, df)
    })
}

pub(super) fn t_dist_rt(x: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df_ok(df))?;
    Ok(t_upper(x, df))
}

pub(super) fn t_dist_2t(x: f64, df: f64) -> Result<f64, ExcelError> {
    domain(x >= 0.0 && df_ok(df))?;
    Ok(2.0 * t_upper(x, df))
}

pub(super) fn t_inv(p: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df_ok(df) && p > 0.0 && p < 1.0)?;
    Ok(if p < 0.5 {
        -t_upper_quantile(p, df)
    } else {
        t_upper_quantile(1.0 - p, df)
    })
}

/// T.INV.2T and TINV: `T.INV(1 - p/2, df)` for `0 < p < 2`.
pub(super) fn t_inv_2t(p: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df_ok(df) && p > 0.0 && p < 2.0)?;
    Ok(if p <= 1.0 {
        t_upper_quantile(p / 2.0, df)
    } else {
        -t_upper_quantile(1.0 - p / 2.0, df)
    })
}

/// CHISQ.DIST; the density at 0 is `#NUM!` for 1 degree of freedom, 1/2 for
/// 2 and 0 above.
pub(super) fn chisq_dist(x: f64, df: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(df_ok(df) && x >= 0.0)?;
    if cumulative {
        return Ok(chisq_tails(x, df).0);
    }
    if x == 0.0 {
        domain(df >= 2.0)?;
        return Ok(if df == 2.0 { 0.5 } else { 0.0 });
    }
    Ok(chisq_density(x, df))
}

pub(super) fn chisq_dist_rt(x: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df_ok(df) && x >= 0.0)?;
    Ok(chisq_tails(x, df).1)
}

pub(super) fn chisq_inv(p: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df >= 1.0 && (0.0..1.0).contains(&p))?;
    Ok(2.0 * gamma_quantile(df / 2.0, p, 1.0 - p))
}

pub(super) fn chisq_inv_rt(p: f64, df: f64) -> Result<f64, ExcelError> {
    domain(df >= 1.0 && p > 0.0 && p <= 1.0)?;
    Ok(2.0 * gamma_quantile(df / 2.0, 1.0 - p, p))
}

/// F.DIST; the density at 0 is `#NUM!` for 1 numerator degree of freedom, 1
/// for 2 and 0 above.
pub(super) fn f_dist(x: f64, d1: f64, d2: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(df_ok(d1) && df_ok(d2) && x >= 0.0)?;
    if cumulative {
        return Ok(f_tails(x, d1, d2).0);
    }
    if x == 0.0 {
        domain(d1 >= 2.0)?;
        return Ok(if d1 == 2.0 { 1.0 } else { 0.0 });
    }
    Ok(f_density(x, d1, d2))
}

pub(super) fn f_dist_rt(x: f64, d1: f64, d2: f64) -> Result<f64, ExcelError> {
    domain(df_ok(d1) && df_ok(d2) && x >= 0.0)?;
    Ok(f_tails(x, d1, d2).1)
}

pub(super) fn f_inv(p: f64, d1: f64, d2: f64) -> Result<f64, ExcelError> {
    domain(df_ok(d1) && df_ok(d2) && (0.0..1.0).contains(&p))?;
    Ok(f_quantile(d1, d2, p, 1.0 - p))
}

pub(super) fn f_inv_rt(p: f64, d1: f64, d2: f64) -> Result<f64, ExcelError> {
    domain(df_ok(d1) && df_ok(d2) && p > 0.0 && p <= 1.0)?;
    Ok(f_quantile(d1, d2, 1.0 - p, p))
}

/// BINOM.DIST and BINOMDIST, with `k` and `n` already truncated.
pub(super) fn binom_dist(k: f64, n: f64, p: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(n >= 0.0 && k >= 0.0 && k <= n && (0.0..=1.0).contains(&p))?;
    Ok(if cumulative {
        binom_cdf(k, n, p)
    } else {
        binom_pmf(k, n, p)
    })
}

/// BINOM.INV and CRITBINOM: the smallest `k` with `P(X <= k) >= alpha`,
/// `n` already truncated; `p` and `alpha` lie strictly between 0 and 1.
pub(super) fn binom_inv(n: f64, p: f64, alpha: f64) -> Result<f64, ExcelError> {
    domain(n >= 0.0 && p > 0.0 && p < 1.0 && alpha > 0.0 && alpha < 1.0)?;
    let mut cumulative = 0.0;
    let mut k = 0.0;
    while k < n {
        cumulative += binom_pmf(k, n, p);
        if cumulative >= alpha {
            return Ok(k);
        }
        k += 1.0;
    }
    Ok(n)
}

/// POISSON.DIST and POISSON, with `k` already truncated.
pub(super) fn poisson_dist(k: f64, mean: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(k >= 0.0 && mean >= 0.0)?;
    Ok(if !cumulative {
        poisson_pmf(k, mean)
    } else if mean == 0.0 {
        1.0
    } else {
        gamma_inc(k + 1.0, mean).1
    })
}

pub(super) fn expon_dist(x: f64, lambda: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(x >= 0.0 && lambda > 0.0)?;
    Ok(if cumulative {
        -(-lambda * x).exp_m1()
    } else {
        lambda * (-lambda * x).exp()
    })
}

pub(super) fn gamma_dist(
    x: f64,
    alpha: f64,
    beta: f64,
    cumulative: bool,
) -> Result<f64, ExcelError> {
    domain(x >= 0.0 && alpha > 0.0 && beta > 0.0)?;
    if cumulative {
        return Ok(gamma_inc(alpha, x / beta).0);
    }
    if x == 0.0 {
        domain(alpha > 1.0)?;
        return Ok(0.0);
    }
    Ok(gamma_density(alpha, x / beta) / beta)
}

pub(super) fn gamma_inv(p: f64, alpha: f64, beta: f64) -> Result<f64, ExcelError> {
    domain(alpha > 0.0 && beta > 0.0 && (0.0..1.0).contains(&p))?;
    Ok(beta * gamma_quantile(alpha, p, 1.0 - p))
}

pub(super) fn weibull_dist(
    x: f64,
    alpha: f64,
    beta: f64,
    cumulative: bool,
) -> Result<f64, ExcelError> {
    domain(x >= 0.0 && alpha > 0.0 && beta > 0.0)?;
    let z = (x / beta).powf(alpha);
    Ok(if cumulative {
        -(-z).exp_m1()
    } else if x == 0.0 {
        0.0
    } else {
        alpha / beta * (x / beta).powf(alpha - 1.0) * (-z).exp()
    })
}

/// BETA.DIST and BETADIST on `[a, b]`.
pub(super) fn beta_dist(
    x: f64,
    alpha: f64,
    beta: f64,
    cumulative: bool,
    a: f64,
    b: f64,
) -> Result<f64, ExcelError> {
    domain(alpha > 0.0 && beta > 0.0 && a < b && x >= a && x <= b)?;
    let width = b - a;
    let (u, v) = ((x - a) / width, (b - x) / width);
    if cumulative {
        return Ok(beta_inc(alpha, beta, u, v).0);
    }
    if x == a {
        domain(alpha > 1.0)?;
        return Ok(0.0);
    }
    if x == b {
        domain(beta > 1.0)?;
        return Ok(0.0);
    }
    Ok(beta_density(alpha, beta, u, v) / width)
}

/// BETA.INV and BETAINV on `[a, b]`.
pub(super) fn beta_inv(p: f64, alpha: f64, beta: f64, a: f64, b: f64) -> Result<f64, ExcelError> {
    domain(p > 0.0 && p < 1.0 && alpha > 0.0 && beta > 0.0 && a < b)?;
    let (x, y) = beta_quantile(alpha, beta, p, 1.0 - p);
    let width = b - a;
    Ok(if x <= 0.5 {
        a + width * x
    } else {
        b - width * y
    })
}

/// NEGBINOM.DIST and NEGBINOMDIST, with the counts already truncated.
pub(super) fn negbinom_dist(f: f64, s: f64, p: f64, cumulative: bool) -> Result<f64, ExcelError> {
    domain(f >= 0.0 && s >= 1.0 && p > 0.0 && p < 1.0)?;
    Ok(if cumulative {
        beta_inc(s, f + 1.0, p, 1.0 - p).0
    } else {
        // C(f + s - 1, s - 1) p^s q^f = s / (f + s) * C(f + s, s) p^s q^f
        s / (f + s) * binom_pmf(s, f + s, p)
    })
}

/// HYPGEOM.DIST and HYPGEOMDIST, with the counts already truncated: a
/// sample count outside the support is a probability of 0 (or 1 above it,
/// cumulatively).
pub(super) fn hypgeom_dist(
    s: f64,
    n: f64,
    m: f64,
    total: f64,
    cumulative: bool,
) -> Result<f64, ExcelError> {
    domain(total > 0.0 && m >= 0.0 && m <= total && n >= 0.0 && n <= total && s >= 0.0)?;
    let low = (n - (total - m)).max(0.0);
    let high = n.min(m);
    if s < low {
        return Ok(0.0);
    }
    if s > high {
        return Ok(if cumulative { 1.0 } else { 0.0 });
    }
    if !cumulative {
        return Ok(hypgeom_pmf(s, n, m, total));
    }
    let mut sum = 0.0;
    let mut i = low;
    while i <= s {
        sum += hypgeom_pmf(i, n, m, total);
        i += 1.0;
    }
    Ok(sum.min(1.0))
}

/* ─────────────────────────── arguments ──────────────────────────── */

/// A number argument.
pub(super) fn number_arg(args: &[ArgumentHandle<'_, '_>], i: usize) -> Result<f64, ExcelError> {
    coerce_num(&scalar_like_value(&args[i])?)
}

/// A count or a number of degrees of freedom: Excel truncates it.
pub(super) fn whole_arg(args: &[ArgumentHandle<'_, '_>], i: usize) -> Result<f64, ExcelError> {
    Ok(number_arg(args, i)?.trunc())
}

/// A cumulative flag: a logical, a blank (FALSE), a number or date
/// (non-zero is TRUE), or the text "TRUE"/"FALSE" in any case.
pub(super) fn logical_arg(args: &[ArgumentHandle<'_, '_>], i: usize) -> Result<bool, ExcelError> {
    match scalar_like_value(&args[i])? {
        value @ (LiteralValue::Text(_)
        | LiteralValue::Boolean(_)
        | LiteralValue::Empty
        | LiteralValue::Error(_)) => crate::coercion::to_logical(&value),
        other => Ok(coerce_num(&other)? != 0.0),
    }
}

/// An optional number: an absent or omitted argument (`BETADIST(x,2,3,0,)`)
/// is `default`.
pub(super) fn optional_arg(
    args: &[ArgumentHandle<'_, '_>],
    i: usize,
    default: f64,
) -> Result<f64, ExcelError> {
    match args.get(i) {
        Some(arg) if !arg.is_omitted() => coerce_num(&scalar_like_value(arg)?),
        _ => Ok(default),
    }
}

/// The value of a distribution function: its number, or its error.
pub(super) fn result<'b>(value: Result<f64, ExcelError>) -> Result<CalcValue<'b>, ExcelError> {
    Ok(CalcValue::Scalar(
        match value.and_then(crate::coercion::sanitize_numeric) {
            Ok(n) => LiteralValue::Number(n),
            Err(e) => LiteralValue::Error(e),
        },
    ))
}

/* ─────────────────────────── compatibility names ──────────────────────────── */

/// Returns the standard normal cumulative distribution, `NORM.S.DIST(z, TRUE)`.
///
/// ```yaml,sandbox
/// title: "Standard normal CDF"
/// formula: "=NORMSDIST(1)"
/// expected: 0.841344746068543
/// ```
#[derive(Debug)]
pub struct NormsDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: NORMSDIST
/// Type: NormsDistLegacyFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: NORMSDIST(arg1: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NormsDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NORMSDIST"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        result(Ok(std_norm_cdf(number_arg(args, 0)?)))
    }
}

/// Returns the standard normal quantile, `NORM.S.INV(probability)`.
///
/// ```yaml,sandbox
/// title: "Two-sided 95% critical value"
/// formula: "=NORMSINV(0.975)"
/// expected: 1.9599639845400536
/// ```
#[derive(Debug)]
pub struct NormsInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: NORMSINV
/// Type: NormsInvLegacyFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: NORMSINV(arg1: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NormsInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NORMSINV"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        result(norm_inv(number_arg(args, 0)?, 0.0, 1.0))
    }
}

/// Returns the normal distribution, `NORM.DIST(x, mean, standard_dev, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Normal CDF"
/// formula: "=NORMDIST(42,40,1.5,TRUE)"
/// expected: 0.9087887802741321
/// ```
#[derive(Debug)]
pub struct NormDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: NORMDIST
/// Type: NormDistLegacyFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: NORMDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NormDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NORMDIST"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let mean = number_arg(args, 1)?;
        let sd = number_arg(args, 2)?;
        let cumulative = logical_arg(args, 3)?;
        result(norm_dist(x, mean, sd, cumulative))
    }
}

/// Returns the normal quantile, `NORM.INV(probability, mean, standard_dev)`.
///
/// ```yaml,sandbox
/// title: "Normal quantile"
/// formula: "=NORMINV(0.5,10,2)"
/// expected: 10
/// ```
#[derive(Debug)]
pub struct NormInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: NORMINV
/// Type: NormInvLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: NORMINV(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NormInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NORMINV"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let p = number_arg(args, 0)?;
        let mean = number_arg(args, 1)?;
        let sd = number_arg(args, 2)?;
        result(norm_inv(p, mean, sd))
    }
}

/// Returns the cumulative lognormal distribution, `LOGNORM.DIST(x, mean, standard_dev, TRUE)`.
///
/// ```yaml,sandbox
/// title: "Lognormal CDF"
/// formula: "=LOGNORMDIST(4,3.5,1.2)"
/// expected: 0.03908355570680048
/// ```
#[derive(Debug)]
pub struct LognormDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: LOGNORMDIST
/// Type: LognormDistLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: LOGNORMDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for LognormDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "LOGNORMDIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let mean = number_arg(args, 1)?;
        let sd = number_arg(args, 2)?;
        result(lognorm_dist(x, mean, sd, true))
    }
}

/// Returns the lognormal quantile, `LOGNORM.INV(probability, mean, standard_dev)`.
///
/// ```yaml,sandbox
/// title: "Lognormal median"
/// formula: "=LOGINV(0.5,0,1)"
/// expected: 1
/// ```
#[derive(Debug)]
pub struct LogInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: LOGINV
/// Type: LogInvLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: LOGINV(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for LogInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "LOGINV"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let p = number_arg(args, 0)?;
        let mean = number_arg(args, 1)?;
        let sd = number_arg(args, 2)?;
        result(lognorm_inv(p, mean, sd))
    }
}

/// Returns the Student's t tail probability for `x >= 0`: one tail
/// (`T.DIST.RT`) or both (`T.DIST.2T`). `deg_freedom` and `tails` are
/// truncated; `tails` must then be 1 or 2, and `deg_freedom` lie in [1, 1E10]
/// (as for every T function).
///
/// ```yaml,sandbox
/// title: "Two-tailed t probability"
/// formula: "=TDIST(2.5,3,2)"
/// expected: 0.08770664700806553
/// ```
#[derive(Debug)]
pub struct TDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: TDIST
/// Type: TDistLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: TDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "TDIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let df = whole_arg(args, 1)?;
        let tails = whole_arg(args, 2)?;
        result(
            domain(x >= 0.0 && (tails == 1.0 || tails == 2.0))
                .and_then(|()| t_dist_rt(x, df))
                .map(|tail| tails * tail),
        )
    }
}

/// Returns the right-tailed chi-square probability, `CHISQ.DIST.RT(x, deg_freedom)`;
/// `deg_freedom` is truncated and must lie in [1, 1E10].
///
/// ```yaml,sandbox
/// title: "Right-tailed chi-square"
/// formula: "=CHIDIST(3,2)"
/// expected: 0.22313016014842982
/// ```
#[derive(Debug)]
pub struct ChiDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: CHIDIST
/// Type: ChiDistLegacyFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: CHIDIST(arg1: number@scalar, arg2: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ChiDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "CHIDIST"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let df = whole_arg(args, 1)?;
        result(chisq_dist_rt(x, df))
    }
}

/// Returns the right-tailed chi-square quantile, `CHISQ.INV.RT(probability, deg_freedom)`.
///
/// ```yaml,sandbox
/// title: "Right-tailed chi-square quantile"
/// formula: "=CHIINV(0.5,2)"
/// expected: 1.3862943611198906
/// ```
#[derive(Debug)]
pub struct ChiInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: CHIINV
/// Type: ChiInvLegacyFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: CHIINV(arg1: number@scalar, arg2: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ChiInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "CHIINV"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let p = number_arg(args, 0)?;
        let df = whole_arg(args, 1)?;
        result(chisq_inv_rt(p, df))
    }
}

/// Returns the right-tailed F probability, `F.DIST.RT(x, deg_freedom1, deg_freedom2)`;
/// the degrees of freedom are truncated and must lie in [1, 1E10].
///
/// ```yaml,sandbox
/// title: "Right-tailed F"
/// formula: "=FDIST(1,2,2)"
/// expected: 0.5
/// ```
#[derive(Debug)]
pub struct FDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: FDIST
/// Type: FDistLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: FDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for FDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FDIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let d1 = whole_arg(args, 1)?;
        let d2 = whole_arg(args, 2)?;
        result(f_dist_rt(x, d1, d2))
    }
}

/// Returns the right-tailed F quantile, `F.INV.RT(probability, deg_freedom1, deg_freedom2)`.
///
/// ```yaml,sandbox
/// title: "Right-tailed F quantile"
/// formula: "=FINV(0.5,2,2)"
/// expected: 1
/// ```
#[derive(Debug)]
pub struct FInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: FINV
/// Type: FInvLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: FINV(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for FInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FINV"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let p = number_arg(args, 0)?;
        let d1 = whole_arg(args, 1)?;
        let d2 = whole_arg(args, 2)?;
        result(f_inv_rt(p, d1, d2))
    }
}

/// Returns the binomial distribution, `BINOM.DIST(number_s, trials, probability_s, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Binomial CDF"
/// formula: "=BINOMDIST(6,10,0.5,TRUE)"
/// expected: 0.828125
/// ```
#[derive(Debug)]
pub struct BinomDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: BINOMDIST
/// Type: BinomDistLegacyFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: BINOMDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for BinomDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "BINOMDIST"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let k = whole_arg(args, 0)?;
        let n = whole_arg(args, 1)?;
        let p = number_arg(args, 2)?;
        let cumulative = logical_arg(args, 3)?;
        result(binom_dist(k, n, p, cumulative))
    }
}

/// Returns the Poisson distribution, `POISSON.DIST(x, mean, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Poisson probability"
/// formula: "=POISSON(2,5,FALSE)"
/// expected: 0.08422433748856833
/// ```
#[derive(Debug)]
pub struct PoissonLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: POISSON
/// Type: PoissonLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: POISSON(arg1: number@scalar, arg2: number@scalar, arg3: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for PoissonLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "POISSON"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let k = whole_arg(args, 0)?;
        let mean = number_arg(args, 1)?;
        let cumulative = logical_arg(args, 2)?;
        result(poisson_dist(k, mean, cumulative))
    }
}

/// Returns the exponential distribution, `EXPON.DIST(x, lambda, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Exponential CDF"
/// formula: "=EXPONDIST(0.2,10,TRUE)"
/// expected: 0.8646647167633873
/// ```
#[derive(Debug)]
pub struct ExponDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: EXPONDIST
/// Type: ExponDistLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: EXPONDIST(arg1: number@scalar, arg2: number@scalar, arg3: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ExponDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "EXPONDIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let lambda = number_arg(args, 1)?;
        let cumulative = logical_arg(args, 2)?;
        result(expon_dist(x, lambda, cumulative))
    }
}

/// Returns the gamma distribution, `GAMMA.DIST(x, alpha, beta, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Gamma CDF"
/// formula: "=GAMMADIST(5,1,1,TRUE)"
/// expected: 0.9932620530009145
/// ```
#[derive(Debug)]
pub struct GammaDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: GAMMADIST
/// Type: GammaDistLegacyFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: GAMMADIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for GammaDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "GAMMADIST"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let alpha = number_arg(args, 1)?;
        let beta = number_arg(args, 2)?;
        let cumulative = logical_arg(args, 3)?;
        result(gamma_dist(x, alpha, beta, cumulative))
    }
}

/// Returns the Weibull distribution, `WEIBULL.DIST(x, alpha, beta, cumulative)`.
///
/// ```yaml,sandbox
/// title: "Weibull CDF"
/// formula: "=WEIBULL(105,20,100,TRUE)"
/// expected: 0.929581390069277
/// ```
#[derive(Debug)]
pub struct WeibullLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: WEIBULL
/// Type: WeibullLegacyFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: WEIBULL(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: logical@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=logical,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for WeibullLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "WEIBULL"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, logical)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let alpha = number_arg(args, 1)?;
        let beta = number_arg(args, 2)?;
        let cumulative = logical_arg(args, 3)?;
        result(weibull_dist(x, alpha, beta, cumulative))
    }
}

/// Returns the cumulative beta distribution on `[A, B]` (default `[0, 1]`),
/// `BETA.DIST(x, alpha, beta, TRUE, A, B)`.
///
/// ```yaml,sandbox
/// title: "Beta CDF on [1, 3]"
/// formula: "=BETADIST(2,8,10,1,3)"
/// expected: 0.6854705810546873
/// ```
#[derive(Debug)]
pub struct BetaDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: BETADIST
/// Type: BetaDistLegacyFn
/// Min args: 3
/// Max args: 5
/// Variadic: false
/// Signature: BETADIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4?: number@scalar, arg5?: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for BetaDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "BETADIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, optional, optional)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let x = number_arg(args, 0)?;
        let alpha = number_arg(args, 1)?;
        let beta = number_arg(args, 2)?;
        let a = optional_arg(args, 3, 0.0)?;
        let b = optional_arg(args, 4, 1.0)?;
        result(beta_dist(x, alpha, beta, true, a, b))
    }
}

/// Returns the beta quantile on `[A, B]` (default `[0, 1]`),
/// `BETA.INV(probability, alpha, beta, A, B)`.
///
/// ```yaml,sandbox
/// title: "Symmetric beta median"
/// formula: "=BETAINV(0.5,2,2)"
/// expected: 0.5
/// ```
#[derive(Debug)]
pub struct BetaInvLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: BETAINV
/// Type: BetaInvLegacyFn
/// Min args: 3
/// Max args: 5
/// Variadic: false
/// Signature: BETAINV(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4?: number@scalar, arg5?: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for BetaInvLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "BETAINV"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, optional, optional)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let p = number_arg(args, 0)?;
        let alpha = number_arg(args, 1)?;
        let beta = number_arg(args, 2)?;
        let a = optional_arg(args, 3, 0.0)?;
        let b = optional_arg(args, 4, 1.0)?;
        result(beta_inv(p, alpha, beta, a, b))
    }
}

/// Returns the hypergeometric probability of exactly `sample_s` successes,
/// `HYPGEOM.DIST(sample_s, number_sample, population_s, number_pop, FALSE)`.
///
/// ```yaml,sandbox
/// title: "Hypergeometric probability"
/// formula: "=HYPGEOMDIST(1,4,8,20)"
/// expected: 0.3632610939112486
/// ```
#[derive(Debug)]
pub struct HypgeomDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: HYPGEOMDIST
/// Type: HypgeomDistLegacyFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: HYPGEOMDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for HypgeomDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "HYPGEOMDIST"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let s = whole_arg(args, 0)?;
        let n = whole_arg(args, 1)?;
        let m = whole_arg(args, 2)?;
        let total = whole_arg(args, 3)?;
        result(hypgeom_dist(s, n, m, total, false))
    }
}

/// Returns the negative binomial probability of `number_f` failures before
/// the `number_s`-th success, `NEGBINOM.DIST(number_f, number_s, probability_s, FALSE)`.
///
/// ```yaml,sandbox
/// title: "Negative binomial probability"
/// formula: "=NEGBINOMDIST(10,5,0.25)"
/// expected: 0.05504866037517785
/// ```
#[derive(Debug)]
pub struct NegbinomDistLegacyFn;
/// [formualizer-docgen:schema:start]
/// Name: NEGBINOMDIST
/// Type: NegbinomDistLegacyFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: NEGBINOMDIST(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NegbinomDistLegacyFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NEGBINOMDIST"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        stats_schema!(num, num, num)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let f = whole_arg(args, 0)?;
        let s = whole_arg(args, 1)?;
        let p = number_arg(args, 2)?;
        result(negbinom_dist(f, s, p, false))
    }
}

pub(super) fn register_builtins() {
    use crate::function_registry::register_builtin;
    use std::sync::Arc;
    register_builtin(Arc::new(NormsDistLegacyFn));
    register_builtin(Arc::new(NormsInvLegacyFn));
    register_builtin(Arc::new(NormDistLegacyFn));
    register_builtin(Arc::new(NormInvLegacyFn));
    register_builtin(Arc::new(LognormDistLegacyFn));
    register_builtin(Arc::new(LogInvLegacyFn));
    register_builtin(Arc::new(TDistLegacyFn));
    register_builtin(Arc::new(ChiDistLegacyFn));
    register_builtin(Arc::new(ChiInvLegacyFn));
    register_builtin(Arc::new(FDistLegacyFn));
    register_builtin(Arc::new(FInvLegacyFn));
    register_builtin(Arc::new(BinomDistLegacyFn));
    register_builtin(Arc::new(PoissonLegacyFn));
    register_builtin(Arc::new(ExponDistLegacyFn));
    register_builtin(Arc::new(GammaDistLegacyFn));
    register_builtin(Arc::new(WeibullLegacyFn));
    register_builtin(Arc::new(BetaDistLegacyFn));
    register_builtin(Arc::new(BetaInvLegacyFn));
    register_builtin(Arc::new(HypgeomDistLegacyFn));
    register_builtin(Arc::new(NegbinomDistLegacyFn));
}
