pub fn compare_floats(a: f64, b: f64, tolerance: f64) {
    let difference = a - b;
    if difference.abs() > tolerance {
        panic!(
            "
                Float comparison failed
                left: {a},
                right: {b},
                difference: {difference}
            "
        )
    }
}

#[macro_export]
macro_rules! assert_nearly_equal {
    (
            $a:expr, $b:expr $(,)?
        ) => {
        $crate::test_utils::compare_floats($a, $b, 1e-6)
    };
    (
            $a: expr, $b:expr, $tolerance:expr $(,)?
        ) => {
        $crate::test_utils::compare_floats($a, $b, $tolerance)
    };
}
