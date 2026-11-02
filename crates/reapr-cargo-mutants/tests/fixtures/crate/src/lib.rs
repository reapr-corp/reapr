pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

/// Caught: tested below.
pub fn is_even(n: u32) -> bool {
    n % 2 == 0
}

/// Missed: no test checks the value.
pub fn double(n: u32) -> u32 {
    n * 2
}

/// Timeout: `n /= 1` never ends the loop.
pub fn countdown(mut n: u32) -> u32 {
    let start = n;
    while n > 0 {
        n -= 1;
    }
    start
}

pub struct Meters(pub u32);

/// Unviable: `Meters` has no `Default`.
pub fn meters(n: u32) -> Meters {
    Meters(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even() {
        assert!(is_even(4));
        assert!(!is_even(3));
    }

    #[test]
    fn double_runs() {
        let _ = double(2);
    }

    #[test]
    fn countdown_returns_start() {
        assert_eq!(countdown(3), 3);
    }

    #[test]
    fn meters_wraps() {
        assert_eq!(meters(2).0, 2);
    }
}
