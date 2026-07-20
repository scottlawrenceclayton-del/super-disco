// bot/src/mobius.rs
//
// Implements the Möbius transformation framework for AMM arbitrage.
//
// A Uniswap V2 swap function f(x) = (γ·R_out·x) / (R_in + γ·x)
// where γ = fee_numerator/fee_denominator (e.g., 997/1000)
//
// This is a Möbius transformation: f(x) = (a·x + b) / (c·x + d)
// represented as the matrix M = [[a, b], [c, d]]
//
// Key insight: composing two swaps = multiplying their matrices.
// A cycle is profitable iff the composed transformation yields
// f(x) > x for some x > 0, which can be checked from the matrix entries.

/// High-precision Möbius transformation using f64.
/// For production, consider using rational arithmetic (num-rational)
/// to avoid floating-point errors on large reserves.
#[derive(Debug, Clone, Copy)]
pub struct MobiusMatrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}

/// Result of analyzing a composed Möbius cycle
#[derive(Debug, Clone)]
pub struct ArbitrageOpportunity {
    pub optimal_input: f64,
    pub expected_output: f64,
    pub expected_profit: f64,
    pub profit_ratio: f64,
}

impl MobiusMatrix {
    /// Create the Möbius matrix for a single Uniswap V2 swap.
    ///
    /// The swap function is:
    ///   f(x) = (γ · R_out · x) / (R_in + γ · x)
    ///
    /// In matrix form M = [[γ · R_out, 0], [γ, R_in]]
    ///
    /// where γ = fee_num / fee_denom (e.g. 997/1000 for 0.3% fee)
    pub fn from_pool(
        reserve_in: f64,
        reserve_out: f64,
        fee_numerator: f64,
        fee_denominator: f64,
    ) -> Self {
        let gamma = fee_numerator / fee_denominator;
        MobiusMatrix {
            a: gamma * reserve_out,
            b: 0.0,
            c: gamma,
            d: reserve_in,
        }
    }

    /// Compose two Möbius transformations via matrix multiplication.
    /// If M1 represents swap1 and M2 represents swap2,
    /// then M2 * M1 represents: first do swap1, then swap2.
    ///
    /// Note: We compose as M_composed = M_last · ... · M_2 · M_1
    /// so that f_composed(x) = f_last(...(f_2(f_1(x))))
    pub fn compose(&self, next: &MobiusMatrix) -> MobiusMatrix {
        // [next] · [self]
        MobiusMatrix {
            a: next.a * self.a + next.b * self.c,
            b: next.a * self.b + next.b * self.d,
            c: next.c * self.a + next.d * self.c,
            d: next.c * self.b + next.d * self.d,
        }
    }

    /// Evaluate the Möbius transformation at input x:
    /// f(x) = (a·x + b) / (c·x + d)
    pub fn evaluate(&self, x: f64) -> f64 {
        (self.a * x + self.b) / (self.c * x + self.d)
    }

    /// Determinant of the matrix: ad - bc
    /// For a single pool: det = γ · R_out · R_in - 0 = γ · R_in · R_out
    /// For a composed cycle back to the same token:
    ///   - The cycle is potentially profitable if a/c > d/b... but for
    ///     cycles starting with b=0 on the first leg, we use a simpler criterion.
    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// For a cycle (composed Möbius transformation where the output token
    /// equals the input token), check if arbitrage is profitable.
    ///
    /// The composed function is f(x) = (a·x + b) / (c·x + d)
    /// We need f(x) > x for some x > 0.
    /// f(x) > x  ⟺  (a·x + b) > x·(c·x + d)  ⟺  -c·x² + (a-d)·x + b > 0
    ///
    /// Since b = 0 when the first leg's matrix has b=0 and we compose:
    /// Actually b might not be 0 after composition. Let's handle the general case.
    ///
    /// Profitability criterion: a > d (when b ≥ 0, c > 0)
    /// More precisely, the function f(x)-x has a maximum, and the max value must be > 0.
    pub fn is_profitable(&self) -> bool {
        // For the cycle f(x) = (ax+b)/(cx+d), profit = f(x) - x
        // profit(x) = (ax + b - cx² - dx) / (cx + d)
        //           = (-cx² + (a-d)x + b) / (cx + d)
        //
        // Numerator is a downward parabola (since c > 0): -cx² + (a-d)x + b
        // Maximum at x* = (a-d)/(2c)
        // Max value = (a-d)²/(4c) + b
        //
        // Profitable iff max value > 0 AND x* > 0
        
        if self.c <= 0.0 {
            return false;
        }

        let a_minus_d = self.a - self.d;
        
        // x* must be positive
        if a_minus_d <= 0.0 && self.b <= 0.0 {
            return false;
        }

        // Check if the parabola's maximum value is positive
        let max_numerator = a_minus_d * a_minus_d / (4.0 * self.c) + self.b;
        max_numerator > 0.0
    }

    /// Compute the optimal input amount that maximizes profit.
    ///
    /// Profit numerator: -c·x² + (a-d)·x + b
    /// This is maximized at x* = (a - d) / (2·c)
    ///
    /// But we also need to ensure this is the OPTIMAL point,
    /// not just the maximum of the parabola — for practical purposes
    /// we want the x that maximizes f(x) - x, and the above is correct.
    ///
    /// However, we also want to find where f(x) = x (break-even points)
    /// and ensure we trade less than the upper break-even.
    pub fn optimal_input(&self) -> Option<ArbitrageOpportunity> {
        if !self.is_profitable() {
            return None;
        }

        let a = self.a;
        let b = self.b;
        let c = self.c;
        let d = self.d;

        // Optimal input: x* = (a - d) / (2c)
        // But if b > 0 (unusual for fresh cycles), we need to also consider that.
        // For most arbitrage cycles starting from token X back to token X
        // through constant-product AMMs, the composed matrix will have b very small
        // or zero. Let's handle both cases.

        let x_star = if c > 0.0 {
            // For the general case with b potentially > 0:
            // The profit function g(x) = f(x) - x = (-cx² + (a-d)x + b) / (cx + d)
            // Taking derivative and setting to 0:
            // g'(x) = 0 leads to: c²x² + 2cdx + d² - (ad - bc) = ... 
            // Actually let's derive properly.
            //
            // g(x) = (ax + b)/(cx + d) - x = (ax + b - cx² - dx)/(cx + d)
            // Let N(x) = -cx² + (a-d)x + b, D(x) = cx + d
            // g'(x) = (N'D - ND') / D²
            // N' = -2cx + (a-d), D' = c
            // g'(x) = ((-2cx + a - d)(cx + d) - (-cx² + (a-d)x + b)·c) / (cx+d)²
            //
            // Numerator:
            // = (-2c²x² - 2cdx + acx + ad - cdx - d²) - (-c²x² + (a-d)cx + bc)
            // = -2c²x² - 2cdx + acx + ad - cdx - d² + c²x² - acx + cdx - bc
            // = -c²x² - 2cdx + ad - d² - bc
            // = -c²x² - 2cdx + (ad - bc) - d²
            //
            // Set to 0: c²x² + 2cdx + d² = ad - bc = det(M)
            // (cx + d)² = det(M)
            // cx + d = sqrt(det(M))  [positive root since cx+d > 0 for x > 0]
            // x = (sqrt(det(M)) - d) / c
            
            let det = a * d - b * c;
            if det <= 0.0 {
                return None;
            }
            let sqrt_det = det.sqrt();
            let x = (sqrt_det - d) / c;
            if x <= 0.0 {
                return None;
            }
            x
        } else {
            return None;
        };

        let output = self.evaluate(x_star);
        let profit = output - x_star;

        if profit <= 0.0 {
            return None;
        }

        Some(ArbitrageOpportunity {
            optimal_input: x_star,
            expected_output: output,
            expected_profit: profit,
            profit_ratio: profit / x_star,
        })
    }
}

/// Compose a sequence of Möbius matrices representing a multi-hop path.
/// Matrices should be in swap order: [hop1, hop2, ..., hopN]
pub fn compose_cycle(matrices: &[MobiusMatrix]) -> MobiusMatrix {
    assert!(!matrices.is_empty());
    let mut composed = matrices[0];
    for m in &matrices[1..] {
        composed = composed.compose(m);
    }
    composed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_swap() {
        // Pool with reserves 1000 ETH and 2,000,000 USDC, 0.3% fee
        let m = MobiusMatrix::from_pool(1000.0, 2_000_000.0, 997.0, 1000.0);
        // Swap 1 ETH
        let out = m.evaluate(1.0);
        // Expected: 997 * 2000000 * 1 / (1000000 + 997) ≈ 1990.03...
        // Wait, let me recalculate: (997 * 2000000) / (1000 * 1000 + 997 * 1) 
        // = 1994000 / (1000000 + 997) = 1994000 / 1000997 ≈ 1992.01
        // Actually: reserve_in = 1000, so d = 1000
        // f(1) = (997*2000000*1) / (997*1 + 1000) = 1994000000/1997 ≈ ... 
        // No wait: f(x) = (a*x + b) / (c*x + d)
        // a = γ*R_out = 0.997 * 2000000 = 1994000
        // b = 0
        // c = γ = 0.997
        // d = R_in = 1000
        // f(1) = 1994000 / (0.997 + 1000) = 1994000 / 1000.997 ≈ 1993.01
        assert!(out > 1990.0 && out < 2000.0);
    }

    #[test]
    fn test_profitable_cycle() {
        // ETH → USDC pool: 100 ETH, 200000 USDC (price = 2000)
        let m1 = MobiusMatrix::from_pool(100.0, 200_000.0, 997.0, 1000.0);
        // USDC → ETH pool (different DEX, cheaper): 250000 USDC, 100 ETH (price = 2500)
        let m2 = MobiusMatrix::from_pool(250_000.0, 100.0, 997.0, 1000.0);

        // Price discrepancy: buy ETH for 2000 USDC, sell for 2500 USDC
        let composed = compose_cycle(&[m1, m2]);
        assert!(composed.is_profitable());

        let opp = composed.optimal_input().unwrap();
        println!("Optimal input: {:.6} ETH", opp.optimal_input);
        println!("Expected output: {:.6} ETH", opp.expected_output);
        println!("Profit: {:.6} ETH", opp.expected_profit);
        assert!(opp.expected_profit > 0.0);
    }

    #[test]
    fn test_no_arb_same_price() {
        // Same pool twice — round trip is always unprofitable due to fees
        let m1 = MobiusMatrix::from_pool(1000.0, 2_000_000.0, 997.0, 1000.0);
        let m2 = MobiusMatrix::from_pool(2_000_000.0, 1000.0, 997.0, 1000.0);
        let composed = compose_cycle(&[m1, m2]);
        // Should not be profitable (fees eat any round-trip)
        let opp = composed.optimal_input();
        // With identical reserves ratio, the round trip loses to fees
        assert!(opp.is_none() || opp.unwrap().expected_profit <= 0.0);
    }

    #[test]
    fn test_three_hop() {
        // ETH → USDC (2000 $/ETH)
        let m1 = MobiusMatrix::from_pool(100.0, 200_000.0, 997.0, 1000.0);
        // USDC → DAI (1:1 but slightly off)
        let m2 = MobiusMatrix::from_pool(100_000.0, 100_500.0, 997.0, 1000.0);
        // DAI → ETH (1950 DAI/ETH - cheaper)
        let m3 = MobiusMatrix::from_pool(195_000.0, 100.0, 997.0, 1000.0);

        let composed = compose_cycle(&[m1, m2, m3]);
        if composed.is_profitable() {
            let opp = composed.optimal_input().unwrap();
            println!("3-hop profit: {:.6} ETH on {:.6} ETH input", opp.expected_profit, opp.optimal_input);
        }
    }
}
