// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

/// See [`crate::metrics::meter`].
#[macro_export]
#[doc(hidden)]
macro_rules! __meter {
    () => {
        $crate::__meter!({})
    };
    ({ $($KEY:ident : $VAL:expr),* $(,)? }) => {
        &$crate::observability::metrics::Meter::get(
            std::env!("CARGO_PKG_NAME"),
            std::env!("CARGO_PKG_VERSION"),
            [
                $((stringify!($KEY), $crate::observability::metrics::Value::from($VAL)).into()),*
            ],
        )
    };
}
pub use __meter;

/// See [`crate::metrics::metric_base`].
#[macro_export]
#[doc(hidden)]
macro_rules! __metric_base {
    // Match the case where we are only given an identifier and qualify it
    ($METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),* $(,)?) => {
        $crate::__metric_base!(type = $METRIC_TYPE, $($BUILD_KEY = $BUILD_VAL),*, {})
    };
    ($METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),*, { $($KEY:ident),* $(,)? }) => {
        $crate::__metric_base!(type = $METRIC_TYPE, $($BUILD_KEY = $BUILD_VAL),*, { $($KEY),* })
    };
    (type = $METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),*, { $($KEY:ident),* $(,)? }) => {{
        $crate::__metric_base!(
            static __STASHED: $crate::observability::metrics::$METRIC_TYPE,
            $($BUILD_KEY = $BUILD_VAL),*,
            { $($KEY),* }
        );
        &*__STASHED
    }};
    (static $CNAME:ident: $METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),*, { $($KEY:ident),* $(,)? }) => {
        $crate::__metric_base!(
            static $CNAME: $crate::observability::metrics::$METRIC_TYPE,
            $($BUILD_KEY = $BUILD_VAL),*,
            { $($KEY),* }
        );
    };
    (static $CNAME:ident: $METRIC_TYPE:ty, $($BUILD_KEY:ident = $BUILD_VAL:expr),*, { $($KEY:ident),* $(,)? }) => {
        static $CNAME: ::std::sync::LazyLock::<$METRIC_TYPE> = ::std::sync::LazyLock::new(|| {
            <$METRIC_TYPE>::builder()
                $(.$BUILD_KEY($BUILD_VAL))*
                .attrs([$(stringify!($KEY),)*] as [&'static str; $crate::__count!($($KEY)*)])
                .build($crate::__meter!())
        });
    };
}
pub use __metric_base;

/// See [`crate::metrics::metric`].
#[macro_export]
#[doc(hidden)]
macro_rules! __metric {
    // Match: identifier and no attributes -- qualify the identifier
    ($METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),* $(,)?) => {
        $crate::__metric!(type = $crate::observability::metrics::$METRIC_TYPE, $($BUILD_KEY = $BUILD_VAL),*, {})
    };
    // Match: identifier and attributes -- qualify the identifier
    ($METRIC_TYPE:ident, $($BUILD_KEY:ident = $BUILD_VAL:expr),* , { $($KEY:ident : $VAL:expr),* $(,)? }) => {
        $crate::__metric!(
            type = $crate::observability::metrics::$METRIC_TYPE,
            $($BUILD_KEY = $BUILD_VAL),*,
            { $($KEY : $VAL,)* }
        )
    };
    // Real implementation of the standard `metric!` usage
    (type = $METRIC_TYPE:ty, $($BUILD_KEY:ident = $BUILD_VAL:expr),* , { $($KEY:ident : $VAL:expr),* $(,)? }) => {{
        use $crate::observability::metrics::MetricBinder;
        static __STASHED: ::std::sync::LazyLock::<<$METRIC_TYPE as MetricBinder>::Bound> = ::std::sync::LazyLock::new(|| {
            <$METRIC_TYPE>::builder()
                $(.$BUILD_KEY($BUILD_VAL))*
                .attrs([$(stringify!($KEY),)*] as [&'static str; $crate::__count!($($KEY)*)])
                .build($crate::__meter!())
                .bind([$((stringify!($KEY), $crate::observability::metrics::Value::from($VAL)).into(),)*]
                    as [$crate::observability::metrics::Attribute; $crate::__count!($($KEY)*)])
        });
        &*__STASHED
    }};
    // === ... from ...
    ($METRIC_TYPE:ident from $BASE:expr $(,)?) => {
        $crate::__metric!($METRIC_TYPE from $BASE, {})
    };
    ($METRIC_TYPE:ident from $BASE:expr, { $($KEY:ident : $VAL:expr),* $(,)? }) => {
        $crate::__metric!(
            from = $BASE,
            type = $crate::observability::metrics::$METRIC_TYPE,
            { $($KEY : $VAL),* }
        )
    };
    (from = $BASE:expr, type = $METRIC_TYPE:ty, { $($KEY:ident : $VAL:expr),* $(,)? }) => {{
        use $crate::observability::metrics::MetricBinder;
        static __STASHED: ::std::sync::LazyLock<<$METRIC_TYPE as MetricBinder>::Bound> = ::std::sync::LazyLock::new(|| {
            $BASE
                .bind([$((stringify!($KEY), $crate::observability::metrics::Value::from($VAL)).into(),)*]
                    as [$crate::observability::metrics::Attribute; $crate::__count!($($KEY)*)])
        });
        &*__STASHED
    }};
}
pub use __metric;

/// Replace a token tree with a substituted expression.
///
/// This is a utility macro used by other macros. See [`__count!`] for an example.
#[macro_export]
#[doc(hidden)]
macro_rules! __replace_expr {
    ($_t:tt $sub:expr) => {
        $sub
    };
}

/// Count the number of token trees.
///
/// ```ignore
/// use coreapi_framework_prototype::__count;
///
/// assert_eq!(0, __count!());
/// assert_eq!(1, __count!(a));
/// assert_eq!(2, __count!(a b));
/// assert_eq!(3, __count!(a [1, 2] c));
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! __count {
    ($($t:tt)*) => { 0usize $(+ $crate::__replace_expr!($t 1))*};
}
