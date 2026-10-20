// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Define metrics and report on them.
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::metric;
//!
//! metric!(Counter, name = "my_counter")
//!     //  ^- the "instrument" type
//!     .add(1);
//! ```
//!
//! The type of operations you can do on a given metric depend on the type of instrument it is. A [`Counter`] metric has
//! [`add`][`GenericBoundCounter::add`] while a [`Gauge`] metric has [`record`][`GenericBoundGauge::record`].
//!
//! # Using the Macros
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::metric;
//!
//! let fuel_gauge = metric!(FloatGauge,
//!     name = "fuel_level",
//!     unit = "gallons",
//!     description = "how much fuel is in the tank",
//! );
//! loop {
//!     fuel_gauge.record(19.3);
//!     break; // you get the idea
//! }
//! ```
//!
//! Metrics can be parameterized with sets of [attributes][`Attribute`], which are [key][`Key`]-[value][`Value`] pairs.
//! This creates a single variable with different annotations. Telemetry dashboards like Graphana allow you to query on
//! these attributes.
//!
//! This is useful when you have multiple instances of the same variable concept; for example, the speed of the wheels
//! on a vehicle. You could create four metrics: `wheel_speed_driver_front`, `wheel_speed_passenger_rear`, and so on,
//! but this is inconvenient. A naive query most likely just wants the `wheel_speed` and _might_ want to narrow in on
//! cases where behavior is aberrant (e.g.: the wheel speeds are very different).
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::metric;
//!
//! let fd_wheel_speed = metric!(
//!     FloatGauge,
//!     name = "wheel_speed",
//!     unit = "rpm",
//!     description = "how fast is this wheel rotating",
//!     {
//!         side: "driver",
//!         longitude: "front",
//!         steers: true,
//!     }
//! );
//! let pr_wheel_speed = metric!(
//!     FloatGauge,
//!     name = "wheel_speed",
//!     unit = "rpm",
//!     description = "how fast is this wheel rotating",
//!     {
//!         side: "passenger",
//!         longitude: "rear",
//!         steers: false,
//!     }
//! );
//! ```
//!
//! > **Attributes vs Separate Variable**
//! >
//! > There is no strict rule on when something should use separate variables versus different parameterizations of the
//! > same variable. Generally, parameterization works well when tracking a member of an instance; the logic of "should
//! > be a field on a `struct`" translates well.
//! >
//! > As an example, the default `grpc_instrument` behavior is to use a single [`CallMetric`] for the service, then
//! > parameterize things like the method name. This helps queries like "Why is the error rate going up?" but allows
//! > narrowing down on a per-method basis.
//!
//! In the last example, you might have noticed a bit of redundancy in the specification. When doing parameterization,
//! it can be helpful to define a basic instrument, then let attributes be filled in later. To do this, use
//! [`metric_base!`] and the `from` syntax in [`metric!`]. To make it accessible,
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::{metric, metric_base};
//!
//! metric_base!(
//!     static BASE: Counter,
//!     name = "the_variable_name",
//!     description = "additional text for people",
//!     { foo }
//! );
//! let bar = metric!(Counter from BASE, { foo: "bar" });
//! let baz = metric!(Counter from BASE, { foo: "baz" });
//! ```
//!
//! A [`metric!`] is static and must be constructed using only constant variables. To get around this, you can always
//! use the regular function calls. Keep in mind that they are a little more inconvenient, but they are far more
//! flexible.
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::{metric_base, BoundFloatGauge, MetricBinder, Value};
//!
//! struct Wheel {
//!     speed_gauge: BoundFloatGauge,
//!     // other things
//! }
//!
//! metric_base!(
//!     static WHEEL_SPEED_BASE: FloatGauge,
//!     name = "wheel_speed",
//!     unit = "rpm",
//!     description = "how fast is this wheel rotating",
//!     { side, longitude, steers }
//! );
//!
//! impl Wheel {
//!     pub fn new(side: &str, front: bool) -> Self {
//!         Self {
//!             speed_gauge: WHEEL_SPEED_BASE.bind([
//!                 ("side", Value::from(side.to_string())),
//!                 ("longitude", Value::from(if front { "front" } else { "rear" })),
//!                 ("steers", Value::from(front)),
//!             ])
//!             // ...
//!         }
//!     }
//! }
//! ```
//!
//! # Instruments
//!
//! ## Histograms
//!
//! When making a histogram, prefer using buckets from the [`HistogramCommonBucket`] enumeration if possible. This
//! unifies buckets across multiple services so they're easier to analyze.
//!
//! ```ignore
//! use coreapi_framework_prototype::metrics::{metric, Histogram, HistogramCommonBucket};
//! use std::time::Instant;
//!
//! let start = Instant::now();
//! // ...do things...
//! metric!(
//!     Histogram,
//!     name = "foo_timer",
//!     buckets = HistogramCommonBucket::Time,
//! ).record(start.elapsed().as_secs_f64());
//! ```

use crate::info_limited;
use opentelemetry::{global, metrics as otel_metrics, InstrumentationScope, KeyValue as OTelKeyValue};
use std::time::Instant;
use std::{borrow::Cow, fmt, marker, sync::OnceLock};

#[macro_use]
mod metrics_macros;

mod poll;
pub use poll::{BoundPollMetric, PollMetric, PollMetricBuilder, WithPollMetric};

/// A logical grouping of instruments.
#[derive(Debug)]
#[cfg_attr(not(doc), repr(transparent))]
pub struct Meter(pub(crate) otel_metrics::Meter);

/// Convenient way of constructing a [`Meter`] with the correct arguments.
///
/// You do not need to call this yourself, as the [`metric!`] and [`metric_base!`] call it for you.
pub use metrics_macros::__meter as meter;

impl Meter {
    /// Get a meter from global configuration.
    ///
    /// See the [`meter!`] macro for easy use.
    ///
    /// ## Arguments
    ///
    /// * `provider`: The name of the library providing instrumentation. The `env!("CARGO_PKG_NAME")` is a good choice
    ///   for this.
    /// * `version`: The version of the library providing instrumentation. This can help distinguish if certain versions
    ///   of the library are having issues. The `env!("CARGO_PKG_VERSION")` is a good choice for this.
    pub fn get(
        provider: impl Into<Cow<'static, str>>,
        version: impl Into<Cow<'static, str>>,
        attributes: impl IntoIterator<Item = Attribute>,
    ) -> Self {
        let scope = InstrumentationScope::builder(provider)
            .with_version(version)
            .with_attributes(attributes.into_iter().map(|attr| attr.0))
            .build();
        let inner = global::meter_provider().meter_with_scope(scope);
        Self(inner)
    }
}

impl AsRef<otel_metrics::Meter> for Meter {
    fn as_ref(&self) -> &otel_metrics::Meter {
        &self.0
    }
}

// =====================================================================================================================
// Metric name prefix

/// Process-wide prefix prepended to every emitted metric name.
///
/// Set once at startup via [`set_metric_prefix`]. Defaults to `"storage"` if a metric is registered before any explicit
/// configuration (preserving the historical naming).
static PREFIX: OnceLock<Cow<'static, str>> = OnceLock::new();

/// Default prefix applied when [`set_metric_prefix`] is never called.
const DEFAULT_PREFIX: &str = "storage";

/// Error returned by [`set_metric_prefix`].
#[derive(Debug)]
pub enum MetricPrefixError {
    /// The metric prefix has already been initialized — either by a prior call to [`set_metric_prefix`] or implicitly by
    /// the first metric being registered with the default prefix. The prefix can only be set once per process.
    AlreadySet,
}

impl fmt::Display for MetricPrefixError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MetricPrefixError::AlreadySet => write!(
                f,
                "metric prefix has already been initialized; call set_metric_prefix once before any metric is registered"
            ),
        }
    }
}

impl std::error::Error for MetricPrefixError {}

/// Set the prefix prepended to every metric name registered after this call.
///
/// For example, with prefix `"myapp"` and a metric declared as `name = "requests"`, the emitted name is
/// `myapp.requests`.
///
/// # Init order
///
/// This must be called *before* any metric is constructed. Metric instruments are lazily registered on first use, so in
/// practice this means calling it during application startup, before serving any traffic. Calling after a metric has
/// been registered (which implicitly locks the default prefix) returns [`MetricPrefixError::AlreadySet`].
///
/// # Separator convention
///
/// OpenTelemetry's [naming guidelines][otel-naming] recommend `.` as the hierarchical separator between segments of a
/// metric name (e.g., `http.server.request.duration`). This library follows that convention by always joining the
/// configured prefix to each metric name with a `.`. Characters *within* your prefix are preserved verbatim, so you
/// are free to use a different separator inside the prefix itself if your application or downstream telemetry stack
/// expects one — `set_metric_prefix("my_app_v2")` produces names like `my_app_v2.requests`.
///
/// [otel-naming]: https://opentelemetry.io/docs/specs/semconv/general/naming/
///
/// # Normalization
///
/// Trailing `.`s on the input are stripped, so `set_metric_prefix("myapp")` and `set_metric_prefix("myapp.")` are
/// equivalent. This makes copy-pasting from existing dotted names painless.
///
/// # Empty prefix
///
/// Passing an empty string (or a string of only `.`s) disables prefixing entirely; metrics are emitted with their bare
/// names. This is occasionally useful for tests or for applications that want full control over the name.
///
/// # Default
///
/// If this is never called, the prefix defaults to `"storage"`, matching the historical behavior.
pub fn set_metric_prefix(prefix: impl Into<Cow<'static, str>>) -> Result<(), MetricPrefixError> {
    let normalized = normalize_prefix(prefix.into());
    PREFIX.set(normalized).map_err(|_| MetricPrefixError::AlreadySet)?;
    let effective = metric_prefix();
    if effective.is_empty() {
        info_limited!("Metric prefix configured (empty) — metrics will be emitted with bare names");
    } else {
        info_limited!(metric_prefix = effective, "Metric prefix configured");
    }
    Ok(())
}

/// Strip trailing `.` characters from a prefix. Empty prefixes (after stripping) are preserved as empty.
fn normalize_prefix(prefix: Cow<'static, str>) -> Cow<'static, str> {
    match prefix {
        Cow::Borrowed(s) => Cow::Borrowed(s.trim_end_matches('.')),
        Cow::Owned(mut s) => {
            while s.ends_with('.') {
                s.pop();
            }
            Cow::Owned(s)
        },
    }
}

/// Return the configured prefix, initializing the global to the default on first read.
///
/// After this returns, [`set_metric_prefix`] will fail with [`MetricPrefixError::AlreadySet`]. If the default is
/// applied here (i.e., no explicit configuration), a one-time info log is emitted so the effective prefix is
/// observable in the application's startup output.
fn metric_prefix() -> &'static str {
    PREFIX.get_or_init(|| {
        info_limited!(
            metric_prefix = DEFAULT_PREFIX,
            "Metric prefix using default (set_metric_prefix was not called before first metric registration)"
        );
        Cow::Borrowed(DEFAULT_PREFIX)
    })
}

/// Apply the configured prefix to a raw metric name. Empty prefixes or empty names are passed through unchanged.
fn apply_prefix(name: Cow<'static, str>) -> Cow<'static, str> {
    apply_prefix_with(metric_prefix(), name)
}

/// Pure prefix-and-name join. Empty prefix or empty name returns `name` unchanged; otherwise joins with `.`.
fn apply_prefix_with(prefix: &str, name: Cow<'static, str>) -> Cow<'static, str> {
    if prefix.is_empty() || name.is_empty() {
        name
    } else {
        Cow::Owned(format!("{prefix}.{name}"))
    }
}

/// Create a metric.
///
/// The general format is to have the type as the first argument, followed by a series of `config = <expr>` entries,
/// then a `{ key: <value> }` block. The type argument is any metric-like entity with a `builder()` function which
/// creates a builder for that metric. The `config = <expr>` entries are transformed into `.config(<expr>)` calls on
/// that builder and any `key` in the attribute block are added as [`attrs`][`InstrumentConfigBuilder::attrs`]. Finally,
/// [`bind`][`MetricBinder::bind`] is called with the `key`-`value` pairs transformed into [`Attribute`]s.
///
/// ## Complete Form
///
/// The most common form of metric definition creates, registers, and binds attributes in a single pass. The metric is
/// constructed lazily once and stored statically, so only the first pass of a `metric!` does the complicated
/// registration process.
///
/// * `metric!($INSTRUMENT, < $BUILD_KEY = $BUILD_VAL, >..., < { < $ATTR_KEY: $ATTR_VAL, >... } >?)`
/// * `metric!(type = $INSTRUMENT, < $BUILD_KEY = $BUILD_VAL, >..., < { < $ATTR_KEY: $ATTR_VAL, >... } >?)`
///
/// Arguments:
///
/// * `$INSTRUMENT`: This names the type of instrument this metric should be. It can be a [`Gauge`], [`Counter`],
///   [`Histogram`], any of their explicitly-typed forms like [`FloatGauge`] or [`UIntHistogram`], an aggregate
///   instrument like a [`CallMetric`], or anything user-defined that matches [`InstrumentBuild`] and [`MetricBinder`]
///   pattern. By default, the `$INSTRUMENT` is qualified by the `coreapi_framework_prototype::metrics` namespace, but
///   you can use the `type =` form to name your instrument type directly.
/// * `$BUILD_KEY = $BUILD_VAL,...`: Optional arguments to set on the instrument itself. There are common build keys
///   like [`name`][`InstrumentConfigBuilder::name`] and [`description`][`InstrumentConfigBuilder::description`], but
///   instruments can have additional options like specifying the [`buckets`][`InstrumentConfigBuilder::buckets`] on a
///   [`Histogram`]. The syntax translates `$key = $val` into a `.$key($val)` call on the builder. Because a `metric!`
///   is static, the `$BUILD_VAL` expression can only refer to things which are also static.
/// * `$ATTR_KEY: $ATTR_VAL,...`: Optional attributes to provide for the metric registration which will appear in the
///   collector output. These are transformed to `("$ATTR_KEY", Value::from($ATTR_VAL))` and provided to the
///   [`bind`][`MetricBinder::bind`] call. While generally more useful in the **From-Base Form** of this macro,
///   attributes provide identifying characteristics to consumers of the metrics.
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::metric;
///
/// let bound_metric = metric!(
///     UIntGauge,                      // <- instrument type
///     name = "foobar_queue_length",   // <- name to appear in collectors
///     description = "The current length of the queue between Foo and Bar",
///     // Arbitrary metadata associated with the metric (optional)
///     {
///         source: "foo",
///         destination: "bar",
///         local: true,
///     }
/// );
///
/// // Usage:
/// bound_metric.record(20);
/// ```
///
/// ## From-Base Form
///
/// This form of the macro takes an existing instrument definition defined with [`metric_base!`] and gives the
/// attributes values. You can not define build parameters here; they should be specified in the `metric_base!` call.
///
/// * `metric!($INSTRUMENT, from = $BASE, < { < $ATTR_KEY: $ATTR_VAL, >... } >?)`
/// * `metric!(type = $INSTRUMENT, from = $BASE, < { < $ATTR_KEY: $ATTR_VAL, >... } >?)`
///
/// Arguments:
///
/// * `$INSTRUMENT`: Same meaning as the **Complete Form** with the caveat that the type here must match the type of
///   instrument `$BASE` is.
/// * `$BASE`: The instrument base, usually defined with [`metric_base!`].
/// * `$ATTR_KEY: $ATTR_VAL,...`: Same meaning as the **Complete Form** with the caveat that all of the keys must match
///   those specified in the `metric_base!` call.
pub use metrics_macros::__metric as metric;

/// Create a base template for metrics with shared configuration.
///
/// This function does all of the work of [`metric!`], but does not actually create the metric itself. It essentially
/// does everything but call the `bind` function. This is useful when creating a common metric with the configuration
/// filled out, but then creating a full one later.
///
/// ## Static Form
///
/// * `metric_base!(static $NAME: $INSTRUMENT, < $BUILD_KEY = $BUILD_VAL, >..., < { < $ATTR_KEY, >... } >?)`
///
/// Arguments:
///
/// * `$NAME`: The identifier name you will use to pass to `from` in a `metric!` definition later. This is different
///   from specifying `name` as a `$BUILD_KEY`, as this name is used inside of the program while the `name` build key is
///   used by consumers. This should match Rust global consant `SCREAMING_SNAKE_CASE` style.
/// * `$INSTRUMENT`: The type of instrument this metric should be. This has the same meaning as in [`metric!`].
/// * `$BUILD_KEY = $BUILD_VAL,...`: Optional arguments to set on the instrument. This has the same meaning as in
///   [`metric!`].
/// * `$ATTR_KEY,...`: Optional attribute names which need to be filled out when used in `metric!`. These have the same
///   meaning as in `metric!` with the change that you can not specify the value in the base; the attribute key list
///   looks like `{ foo, bar, baz }`, but the metric will have `{ foo: "Gurg", bar: true, baz: 5 }`.
///
/// ## Anonymous Form
///
/// The anonymous form does everything the **Static Form** does, but does not bind the created value to a global static
/// name. As such, it can not be used in the `metric!` macro, as that requires things to be in static space. However,
/// the returned value is still a base for an instrument, so you can call [`bind`][`MetricBinder::bind`] on it. This is
/// useful for creating metrics with attributes that are dynamically-bound.
///
/// * `metric_base!($INSTRUMENT, < $BUILD_KEY = $BUILD_VAL, >..., < { < $ATTR_KEY, >... } >?)`
/// * `metric_base!(type = $INSTRUMENT, < $BUILD_KEY = $BUILD_VAL, >..., < { < $ATTR_KEY, >... } >?)`
///
/// All of the arguments to this form have the same meaning as the **Static Form** with the omission of `$NAME`.
pub use metrics_macros::__metric_base as metric_base;

// =====================================================================================================================
// Attributes, Bindings, Etc. -- the top level concepts.

/// A metric binder takes a configured instrument and binds the required attributes to it.
pub trait MetricBinder {
    /// The bound form of the metric; i.e.: the [`BoundGauge`] to [`Gauge`].
    type Bound;

    /// Attempt to bind this metric with the `attributes`.
    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError>;

    /// Like [`try_bind`][`MetricBinder::try_bind`], but always either panic or create a binding.
    ///
    /// While the default implementation panics on failure, implementations are encouraged to provide a fallback metric
    /// when debug assertions are disabled.
    fn bind<A: Into<Attribute>>(&self, attributes: impl IntoIterator<Item = A>) -> Self::Bound {
        self.try_bind(attributes).unwrap()
    }
}

/// Error returned when [`try_bind`][`MetricBinder::try_bind`] fails to bind.
#[derive(Debug)]
pub struct MetricBindError {
    message: Cow<'static, str>,
}

impl From<&'static str> for MetricBindError {
    fn from(value: &'static str) -> Self {
        Self {
            message: Cow::Borrowed(value),
        }
    }
}

impl From<String> for MetricBindError {
    fn from(value: String) -> Self {
        Self {
            message: Cow::Owned(value),
        }
    }
}

impl fmt::Display for MetricBindError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MetricBindError {}

/// Build the configuration for an instrument.
///
/// This type provides builder calls like [`name`][`InstrumentConfigBuilder::name`] and
/// [`description`][`InstrumentConfigBuilder::description`] to fill out the parameters of a configuration. When a user
/// is done configuring, they call [`build`][`InstrumentConfigBuilder::build`] to create the instrument.
pub struct InstrumentConfigBuilder<Instrument, Config> {
    config: Config,
    _instrument: marker::PhantomData<Instrument>,
}

impl<Instrument, Config: Default> Default for InstrumentConfigBuilder<Instrument, Config> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            _instrument: marker::PhantomData,
        }
    }
}

pub trait InstrumentBuild: Sized {
    type Config;

    /// Attempt to create and register an instrument with the given `meter` and `config`.
    ///
    /// See [`InstrumentConfigBuilder::build`] and [`InstrumentConfigBuilder::try_build`].
    fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError>;
}

impl<Instrument, Config: HasConfigCommon> InstrumentConfigBuilder<Instrument, Config> {
    /// Set the name of this instrument.
    ///
    /// The name should be unique and recognizable. The recommended style is `snake_case`. While providing the name is
    /// not required, not providing a name will cause the system to generate one for you, which is unlikely to be
    /// useful.
    pub fn name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.config.get_mut().name = Some(name.into());
        self
    }

    /// Describe the units of this instrument.
    ///
    /// This can be any descriptive string, but try to stick to SI units. For example, `"ms"`, `"s"`, `"kB"`, etc. Note
    /// that this is case-sensitive, so `"kB"` and `"kb"` are different.
    pub fn unit(mut self, unit: impl Into<Cow<'static, str>>) -> Self {
        self.config.get_mut().unit = Some(unit.into());
        self
    }

    /// A user-facing description of this item.
    pub fn description(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.config.get_mut().description = Some(description.into());
        self
    }

    /// Set the names of the attributes which must be bound to.
    pub fn attrs<K: Into<Key>>(mut self, attrs: impl IntoIterator<Item = K>) -> Self {
        self.config.get_mut().attributes = attrs.into_iter().map(Into::into).collect();
        self
    }

    /// Try to build this instrument and register it with the given `meter`.
    pub fn try_build(self, meter: &Meter) -> Result<Instrument, InstrumentBuildError>
    where
        Instrument: InstrumentBuild<Config = Config>,
    {
        Instrument::try_build(meter, self.config)
    }

    /// Build this instrument and register it with the given `meter`.
    ///
    /// After a user has done all the configuration with `name`, `unit`, `attrs`, and so on (possibly through use of the
    /// [`metric!`] or [`metric_base!`] macros), this function ties everything together by registering it with the
    /// `meter`.
    pub fn build(self, meter: &Meter) -> Instrument
    where
        Instrument: InstrumentBuild<Config = Config>,
    {
        self.try_build(meter).unwrap()
    }
}

/// Contains configuration common for all instruments.
///
/// See [`HasConfigCommon`].
#[derive(Clone, Default)]
pub struct InstrumentConfigCommon {
    pub name: Option<Cow<'static, str>>,
    pub unit: Option<Cow<'static, str>>,
    pub description: Option<Cow<'static, str>>,
    pub attributes: Vec<Key>,
}

impl InstrumentConfigCommon {
    /// Return the name with the configured process-wide prefix applied.
    ///
    /// See [`set_metric_prefix`] for how the prefix is configured. If no name has been set on this configuration, this
    /// returns the empty string (the prefix is not applied to empty names).
    pub(crate) fn resolved_name(&self) -> Cow<'static, str> {
        apply_prefix(self.name.clone().unwrap_or(Cow::Borrowed("")))
    }

    /// Imbue the OpenTelemetry builder with this configuration.
    ///
    /// This function is specific to building OpenTelemetry metrics.
    pub fn imbue_otel<T>(
        self,
        otel_builder: otel_metrics::InstrumentBuilder<T>,
    ) -> (otel_metrics::InstrumentBuilder<T>, Vec<Key>) {
        let mut otel_builder = otel_builder;
        if let Some(unit) = self.unit {
            otel_builder = otel_builder.with_unit(unit);
        }
        if let Some(description) = self.description {
            otel_builder = otel_builder.with_description(description);
        }

        (otel_builder, self.attributes)
    }

    /// An exact copy of `imbue_otel` above because the OpenTelemtry API split these things.
    ///
    /// ### TODO
    /// This should have the contents merged with `imbue_otel`. A macro would do the trick, but using OpenTelemetry
    /// traits or some form of unification here would be preferred. A histogram could be built from a regular
    /// `InstrumentBuilder` before 0.26, so maybe that will happen again.
    pub fn imbue_otel_histogram<T>(
        self,
        otel_builder: otel_metrics::HistogramBuilder<T>,
    ) -> (otel_metrics::HistogramBuilder<T>, Vec<Key>) {
        let mut otel_builder = otel_builder;
        if let Some(unit) = self.unit {
            otel_builder = otel_builder.with_unit(unit);
        }
        if let Some(description) = self.description {
            otel_builder = otel_builder.with_description(description);
        }

        (otel_builder, self.attributes)
    }
}

/// Provides access to common configuration options.
///
/// This is implemented by configuration types which have the [`InstrumentConfigCommon`] and additional properties. It
/// is used by the [`InstrumentConfigBuilder`] to mutably access that common configuration when a user calls a builder
/// function.
pub trait HasConfigCommon {
    fn get_mut(&mut self) -> &mut InstrumentConfigCommon;
}

impl HasConfigCommon for InstrumentConfigCommon {
    fn get_mut(&mut self) -> &mut InstrumentConfigCommon {
        self
    }
}

/// See [`InstrumentBuild::try_build`].
#[derive(Debug)]
pub struct InstrumentBuildError {
    message: Cow<'static, str>,
}

impl From<&'static str> for InstrumentBuildError {
    fn from(value: &'static str) -> Self {
        Self {
            message: Cow::Borrowed(value),
        }
    }
}

impl From<String> for InstrumentBuildError {
    fn from(value: String) -> Self {
        Self {
            message: Cow::Owned(value),
        }
    }
}

impl fmt::Display for InstrumentBuildError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InstrumentBuildError {}

/// An attribute is a [key][`Key`]-[value][`Value`] pair.
#[derive(Clone, PartialEq)]
#[cfg_attr(not(doc), repr(transparent))]
pub struct Attribute(OTelKeyValue);

impl Attribute {
    pub fn new(key: impl Into<Key>, value: impl Into<Value>) -> Self {
        Self(opentelemetry::KeyValue::new(key.into().0, value.into().0))
    }

    pub fn key(&self) -> &Key {
        unsafe { std::mem::transmute(&self.0.key) }
    }

    pub fn value(&self) -> &Value {
        unsafe { std::mem::transmute(&self.0.value) }
    }
}

impl fmt::Debug for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Attribute(key={}, value={})", self.key(), self.value())
    }
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}={}", self.key(), self.value())
    }
}

impl<K: Into<Key>, V: Into<Value>> From<(K, V)> for Attribute {
    fn from(value: (K, V)) -> Self {
        Self::new(value.0, value.1)
    }
}

/// A string key of an [`Attribute`].
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(not(doc), repr(transparent))]
pub struct Key(opentelemetry::Key);

impl Key {
    pub fn new(value: impl Into<Self>) -> Self {
        value.into()
    }

    /// Create a key from a static string.
    ///
    /// This is useful for creating keys which are known at compile time.
    pub const fn from_static_str(value: &'static str) -> Self {
        Self(opentelemetry::Key::from_static_str(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Implement the `From<$from> for $goal` for types which are easily-convertible into their OpenTelemetry counterparts.
macro_rules! impl_conversion_otel {
    ($goal:ident, $from:ty) => {
        impl From<$from> for $goal {
            fn from(value: $from) -> Self {
                Self(opentelemetry::$goal::from(value))
            }
        }
    };
    ($goal:ident, $from:ty, $($rem:ty),+) => {
        impl_conversion_otel!($goal, $from);
        impl_conversion_otel!($goal, $($rem),+);
    }
}

impl_conversion_otel!(Key, String, &'static str, Cow<'static, str>);

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Key({self})")
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, PartialEq)]
#[cfg_attr(not(doc), repr(transparent))]
pub struct Value(opentelemetry::Value);

impl Value {
    pub fn new(value: impl Into<Self>) -> Self {
        value.into()
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Value({self})")
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_conversion_otel!(Value, bool, i64, f64, String, &'static str, Cow<'static, str>);

/// Attempt to bind the instrument-specified `keys` to the user-provided `attrs`.
///
/// This function is used by the [`MetricBinder`] implementations of instruments.
pub fn try_bind_attributes<A: Into<Attribute>>(
    keys: &[Key],
    attrs: impl IntoIterator<Item = A>,
) -> Result<Vec<Attribute>, MetricBindError> {
    // NOTE: There is room for optimization in this function, but there should not be that many attributes so it is
    //       probably not a problem.
    let mut bound_attrs: Vec<Option<Attribute>> = std::iter::repeat_n(None, keys.len()).collect();
    let mut errs: Vec<String> = Vec::new();

    for attr in attrs.into_iter().map(Into::into) {
        let Some(idx) = keys.iter().position(|key| key == attr.key()) else {
            errs.push(format!("unrecognized attribute name `{}`", attr.key()));
            continue;
        };

        let bound_attr = &mut bound_attrs[idx];
        if bound_attr.is_some() {
            errs.push(format!("attribute `{}` specified more than once", attr.key()));
        }
        *bound_attr = Some(attr);
    }

    let bound_attrs = bound_attrs
        .into_iter()
        .enumerate()
        .filter_map(|(idx, maybe_attr)| {
            if maybe_attr.is_none() {
                errs.push(format!("attribute `{}` was not specified", keys[idx]));
            }
            maybe_attr
        })
        .collect();

    if !errs.is_empty() {
        if errs.len() == 1 {
            return Err(MetricBindError::from(errs[0].clone()));
        } else {
            return Err(MetricBindError::from(format!("multiple errors: {}", errs.join(", "))));
        }
    }

    Ok(bound_attrs)
}

// =====================================================================================================================
// Counter

/// A [counter](https://opentelemetry.io/docs/specs/otel/metrics/api/#counter) which accumulates monotonically.
///
/// Counters provide the [`add`][`GenericBoundCounter::add`] function, which accepts the a value and adds it to a global
/// accumulator. The default type of observation is a `u64`, but `f64`s can be observed by using a [`FloatCounter`].
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::metric;
///
/// metric!(Counter, name = "uint_counter")
///     .add(5);
/// metric!(UIntCounter, name = "uint_counter_explicit")
///     .add(5);
/// metric!(FloatCounter, name = "float_counter")
///     .add(4.2);
/// ```
pub struct GenericCounter<T> {
    underlying: otel_metrics::Counter<T>,
    keys: Vec<Key>,
}

impl<T> GenericCounter<T> {
    pub fn builder() -> InstrumentConfigBuilder<Self, InstrumentConfigCommon> {
        InstrumentConfigBuilder::default()
    }
}

pub type Counter = GenericCounter<u64>;
pub type UIntCounter = GenericCounter<u64>;
pub type FloatCounter = GenericCounter<f64>;

macro_rules! impl_instrument_build_counter {
    ($TYPE:ty, $OTEL_CALL:ident) => {
        impl InstrumentBuild for GenericCounter<$TYPE> {
            type Config = InstrumentConfigCommon;

            fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
                let name = config.resolved_name();
                let (builder, keys) = config.imbue_otel(meter.0.$OTEL_CALL(name));

                Ok(Self {
                    underlying: builder.build(),
                    keys,
                })
            }
        }
    };
}
impl_instrument_build_counter!(u64, u64_counter);
impl_instrument_build_counter!(f64, f64_counter);

impl<T: Clone + Default + std::fmt::Display> MetricBinder for GenericCounter<T> {
    type Bound = GenericBoundCounter<T>;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes = try_bind_attributes(&self.keys, attributes)?;
        let attributes: Vec<OTelKeyValue> = attributes.into_iter().map(|x| x.0).collect();
        self.underlying.add(T::default(), &attributes);
        Ok(Self::Bound {
            underlying: self.underlying.clone(),
            attributes,
        })
    }
}

#[derive(Clone)]
pub struct GenericBoundCounter<T> {
    underlying: otel_metrics::Counter<T>,
    attributes: Vec<OTelKeyValue>,
}

pub type BoundCounter = GenericBoundCounter<u64>;
pub type BoundUIntCounter = GenericBoundCounter<u64>;
pub type BoundFloatCounter = GenericBoundCounter<f64>;

impl<T> GenericBoundCounter<T> {
    pub fn add(&self, value: T) {
        self.underlying.add(value, &self.attributes)
    }
}

// =====================================================================================================================
// Gauge

/// A [gauge](https://opentelemetry.io/docs/specs/otel/metrics/api/#gauge) which records values without accumulation.
///
/// Guages provide the [`record`][`GenericBoundGauge::record`] function, which accepts the a value for record. The
/// default type of observation is a `u64`, but `f64`s can be observed by using a [`FloatGauge`].
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::metric;
///
/// metric!(Gauge, name = "uint_gauge")
///     .record(5);
/// metric!(UIntGauge, name = "uint_gauge_explicit")
///     .record(5);
/// metric!(FloatGauge, name = "float_gauge")
///     .record(4.2);
/// ```
pub struct GenericGauge<T> {
    underlying: otel_metrics::Gauge<T>,
    keys: Vec<Key>,
}

impl<T> GenericGauge<T> {
    pub fn builder() -> InstrumentConfigBuilder<Self, InstrumentConfigCommon> {
        InstrumentConfigBuilder::default()
    }
}

pub type Gauge = GenericGauge<u64>;
pub type IntGauge = GenericGauge<i64>;
pub type UIntGauge = GenericGauge<u64>;
pub type FloatGauge = GenericGauge<f64>;

macro_rules! impl_instrument_build_gauge {
    ($TYPE:ty, $OTEL_CALL:ident) => {
        impl InstrumentBuild for GenericGauge<$TYPE> {
            type Config = InstrumentConfigCommon;

            fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
                let name = config.resolved_name();
                let (builder, keys) = config.imbue_otel(meter.0.$OTEL_CALL(name));

                Ok(Self {
                    underlying: builder.build(),
                    keys,
                })
            }
        }
    };
}
impl_instrument_build_gauge!(u64, u64_gauge);
impl_instrument_build_gauge!(i64, i64_gauge);
impl_instrument_build_gauge!(f64, f64_gauge);

impl<T: Clone> MetricBinder for GenericGauge<T> {
    type Bound = GenericBoundGauge<T>;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes = try_bind_attributes(&self.keys, attributes)?;
        Ok(Self::Bound {
            underlying: self.underlying.clone(),
            attributes: attributes.into_iter().map(|x| x.0).collect(),
        })
    }
}

#[derive(Clone)]
pub struct GenericBoundGauge<T> {
    underlying: otel_metrics::Gauge<T>,
    attributes: Vec<OTelKeyValue>,
}

pub type BoundGauge = GenericBoundGauge<u64>;
pub type BoundIntGauge = GenericBoundGauge<i64>;
pub type BoundUIntGauge = GenericBoundGauge<u64>;
pub type BoundFloatGauge = GenericBoundGauge<f64>;

impl<T> GenericBoundGauge<T> {
    pub fn record(&self, value: T) {
        self.underlying.record(value, &self.attributes)
    }
}

// =====================================================================================================================
// Histogram

/// A [histogram](https://opentelemetry.io/docs/specs/otel/metrics/api/#histogram) tracks buckets of measurements that
/// might be statistically meaningful.
///
/// Histograms provide the [`record`][`GenericBoundHistogram::record`] function, which categorizes the value by placing
/// it into a bucket and accumulates the count of that bucket. The default type of observation is a `f64`, but `u64`s
/// can be observed by using a [`UIntHistogram`].
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::metric;
///
/// let histogram = metric!(Histogram, name = "my_histogram", buckets = [1.0, 2.0, 5.0]);
/// histogram.record(0.5); // <- bucket 0: it is before 1.0
/// histogram.record(1.0); // <- bucket 0 (but don't rely on this for floats)
/// histogram.record(1.5); // <- bucket 1
/// histogram.record(9.0); // <- bucket 3: past the end
/// ```
pub struct GenericHistogram<T> {
    underlying: otel_metrics::Histogram<T>,
    keys: Vec<Key>,
}

impl<T> GenericHistogram<T> {
    pub fn builder() -> InstrumentConfigBuilder<Self, HistogramConfig<T>> {
        InstrumentConfigBuilder::default()
    }

    /// Get the keys required for binding this histogram.
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    /// Get a reference to the underlying OpenTelemetry histogram.
    pub(crate) fn underlying(&self) -> &otel_metrics::Histogram<T> {
        &self.underlying
    }
}

pub type Histogram = GenericHistogram<f64>;
pub type FloatHistogram = GenericHistogram<f64>;
pub type UIntHistogram = GenericHistogram<u64>;

pub struct HistogramConfig<T> {
    common: InstrumentConfigCommon,
    buckets: Option<HistogramBuckets>,
    _report: marker::PhantomData<T>,
}

impl<Instrument, T> InstrumentConfigBuilder<Instrument, HistogramConfig<T>> {
    /// Set the bucket boundaries to capture in this histogram.
    pub fn buckets(mut self, spec: impl Into<HistogramBuckets>) -> Self {
        self.config.buckets = Some(spec.into());
        self
    }
}

impl<T> Default for HistogramConfig<T> {
    fn default() -> Self {
        Self {
            common: Default::default(),
            buckets: None,
            _report: marker::PhantomData,
        }
    }
}

impl<T> HasConfigCommon for HistogramConfig<T> {
    fn get_mut(&mut self) -> &mut InstrumentConfigCommon {
        &mut self.common
    }
}

macro_rules! impl_instrument_build_histogram {
    ($TYPE:ty, $OTEL_CALL:ident) => {
        impl InstrumentBuild for GenericHistogram<$TYPE> {
            type Config = HistogramConfig<$TYPE>;

            fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
                let name = config.common.resolved_name();
                let (mut builder, keys) = config.common.imbue_otel_histogram(meter.0.$OTEL_CALL(name));
                if let Some(buckets) = config.buckets {
                    builder = builder.with_boundaries(buckets.into_boundaries());
                }

                Ok(Self {
                    underlying: builder.build(),
                    keys,
                })
            }
        }
    };
}
impl_instrument_build_histogram!(u64, u64_histogram);
impl_instrument_build_histogram!(f64, f64_histogram);

/// A histogram that records `T::default()` when bound to initialize the metric.
///
/// This is useful for ensuring metrics appear in dashboards even before any real values are recorded.
/// See the Grafana issue: <https://github.com/prometheus/prometheus/issues/1673>
pub struct InitializedHistogram<T> {
    inner: GenericHistogram<T>,
}

impl<T> InitializedHistogram<T> {
    pub fn builder() -> InstrumentConfigBuilder<Self, HistogramConfig<T>> {
        InstrumentConfigBuilder::default()
    }
}

pub type InitializedHistogramF64 = InitializedHistogram<f64>;
pub type InitializedHistogramU64 = InitializedHistogram<u64>;

macro_rules! impl_instrument_build_initialized_histogram {
    ($TYPE:ty, $OTEL_CALL:ident) => {
        impl InstrumentBuild for InitializedHistogram<$TYPE> {
            type Config = HistogramConfig<$TYPE>;

            fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
                Ok(Self {
                    inner: GenericHistogram::try_build(meter, config)?,
                })
            }
        }
    };
}
impl_instrument_build_initialized_histogram!(u64, u64_histogram);
impl_instrument_build_initialized_histogram!(f64, f64_histogram);

impl<T: Clone + Default> MetricBinder for InitializedHistogram<T> {
    type Bound = GenericBoundHistogram<T>;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes = try_bind_attributes(self.inner.keys(), attributes)?;
        let attributes = attributes.into_iter().map(|x| x.0).collect::<Vec<_>>();
        self.inner.underlying().record(T::default(), &attributes);
        Ok(Self::Bound {
            underlying: self.inner.underlying().clone(),
            attributes,
        })
    }
}

impl<T: Clone> MetricBinder for GenericHistogram<T> {
    type Bound = GenericBoundHistogram<T>;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes = try_bind_attributes(&self.keys, attributes)?;
        let attributes = attributes.into_iter().map(|x| x.0).collect::<Vec<_>>();
        Ok(Self::Bound {
            underlying: self.underlying.clone(),
            attributes,
        })
    }
}

#[derive(Clone)]
pub struct GenericBoundHistogram<T> {
    underlying: otel_metrics::Histogram<T>,
    attributes: Vec<OTelKeyValue>,
}

pub type BoundHistogram = GenericBoundHistogram<f64>;
pub type BoundFloatHistogram = GenericBoundHistogram<f64>;
pub type BoundUIntHistogram = GenericBoundHistogram<u64>;

impl<T> GenericBoundHistogram<T> {
    pub fn record(&self, value: T) {
        self.underlying.record(value, &self.attributes)
    }
}

/// Define boundaries for `Histogram` metrics.
pub struct HistogramBuckets {
    inner: HistogramBucketsInner,
}

impl HistogramBuckets {
    /// Create a new set of buckets with the given `boundaries`.
    pub fn new(boundaries: Vec<f64>) -> Self {
        Self {
            inner: HistogramBucketsInner::Custom(boundaries),
        }
    }

    /// Create a set of buckets with custom `boundaries`.
    ///
    /// Internally, historgram bucked boundaries are always stored as `f64`s. This method performs any conversions for
    /// you.
    pub fn with_custom_boundaries<T: Into<f64>>(boundaries: impl IntoIterator<Item = T>) -> Self {
        Self {
            inner: HistogramBucketsInner::Custom(boundaries.into_iter().map(Into::into).collect()),
        }
    }

    /// Get the bucket boundaries.
    pub fn boundaries(&self) -> &[f64] {
        match self.inner {
            HistogramBucketsInner::Common(known) => known.boundaries(),
            HistogramBucketsInner::Custom(ref vals) => vals,
            HistogramBucketsInner::CustomStatic(vals) => vals,
        }
    }

    /// Get the bucket boundaries as a `Vec<f64>`.
    pub fn into_boundaries(self) -> Vec<f64> {
        match self.inner {
            HistogramBucketsInner::Common(known) => known.boundaries().to_owned(),
            HistogramBucketsInner::Custom(vals) => vals,
            HistogramBucketsInner::CustomStatic(vals) => vals.to_owned(),
        }
    }
}

impl From<HistogramCommonBucket> for HistogramBuckets {
    fn from(value: HistogramCommonBucket) -> Self {
        Self {
            inner: HistogramBucketsInner::Common(value),
        }
    }
}

impl From<&'static [f64]> for HistogramBuckets {
    fn from(value: &'static [f64]) -> Self {
        Self {
            inner: HistogramBucketsInner::CustomStatic(value),
        }
    }
}

impl<T: Into<f64>, const N: usize> From<[T; N]> for HistogramBuckets {
    fn from(value: [T; N]) -> Self {
        Self::with_custom_boundaries(value)
    }
}

impl From<Vec<f64>> for HistogramBuckets {
    fn from(value: Vec<f64>) -> Self {
        Self::new(value)
    }
}

impl<'a> From<&'a [u64]> for HistogramBuckets {
    fn from(value: &'a [u64]) -> Self {
        Self::with_custom_boundaries(value.iter().map(|x| *x as f64))
    }
}

#[derive(Clone, Debug)]
enum HistogramBucketsInner {
    Common(HistogramCommonBucket),
    Custom(Vec<f64>),
    CustomStatic(&'static [f64]),
}

/// Access commonly used histogram buckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistogramCommonBucket {
    /// Used for data size histograms (e.g.: size of a file).
    Data,
    /// Used for timing histograms.
    Time,
    /// Used when counting the number of times a method has been called (e.g.: gRPC method invocation counts).
    CallCount,
}

impl HistogramCommonBucket {
    pub fn boundaries(&self) -> &'static [f64] {
        match self {
            Self::Data => {
                const KIB: usize = 1024;
                const MIB: usize = KIB * KIB;
                const GIB: usize = MIB * KIB;
                &[
                    KIB as f64,
                    (2 * KIB) as f64,
                    (4 * KIB) as f64,
                    (8 * KIB) as f64,
                    (16 * KIB) as f64,
                    (32 * KIB) as f64,
                    (64 * KIB) as f64,
                    (128 * KIB) as f64,
                    (256 * KIB) as f64,
                    (512 * KIB) as f64,
                    MIB as f64,
                    (4 * MIB) as f64,
                    (16 * MIB) as f64,
                    (64 * MIB) as f64,
                    (256 * MIB) as f64,
                    GIB as f64,
                    (4 * GIB) as f64,
                    (16 * GIB) as f64,
                    (64 * GIB) as f64,
                    (256 * GIB) as f64,
                ]
            },
            // Buckets for latency measured in seconds. This is a modification of the suggested values for gRPC to
            // support longer latencies, as seen in Omniverse usage.
            // Suggested gRPC values and example usage:
            // https://github.com/grpc/proposal/blob/master/A66-otel-stats.md#units
            // https://github.com/grpc/grpc/blob/724fa60db4e6ea23de14d8763efcf090323a5ec0/examples/cpp/otel/util.cc#L56
            Self::Time => &[
                0.0, 0.0001, 0.0005, 0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.008, 0.01, 0.013, 0.016, 0.02, 0.025,
                0.03, 0.04, 0.05, 0.065, 0.08, 0.1, 0.13, 0.16, 0.2, 0.25, 0.3, 0.4, 0.5, 0.65, 0.8, 1.0, 2.0, 5.0,
                10.0, 20.0, 50.0, 100.0, 300.0, 600.0, 1000.0,
            ],
            Self::CallCount => &[
                0f64, 1f64, 2f64, 4f64, 8f64, 16f64, 32f64, 64f64, 128f64, 256f64, 512f64, 1024f64, 2048f64, 4096f64,
                8192f64, 16384f64, 32768f64, 65536f64,
            ],
        }
    }
}

// =====================================================================================================================
// CallMetric

/// Metrics common to an RPC call.
///
/// This is an aggregate metric which tracks counts of calls, the number of times the calls complete in success and
/// error, and tracks a histogram of timings.
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::metric;
///
/// pub fn check(contents: &str) -> Result<usize, String> {
///     let stat_tracker = metric!(CallMetric, name = "check_api", { service: "MyServiceName", method: "check" })
///         .enter();
///
///     if contents.len() % 2 == 0 {
///         stat_tracker.succeed_with(contents.len())
///     } else {
///         stat_tracker.error_with(format!("string should be an even number, but has length {}", contents.len()))
///     }
/// }
///
/// assert!(check("foo").is_err());
/// assert!(check("taco").is_ok());
/// ```
pub struct CallMetric {
    call: UIntCounter,
    success: UIntCounter,
    error: UIntCounter,
    time: Histogram,
}

impl CallMetric {
    pub fn builder() -> CallMetricBuilder {
        CallMetricBuilder::default()
    }
}

#[derive(Default)]
pub struct CallMetricBuilder {
    call: InstrumentConfigBuilder<UIntCounter, InstrumentConfigCommon>,
    success: InstrumentConfigBuilder<UIntCounter, InstrumentConfigCommon>,
    error: InstrumentConfigBuilder<UIntCounter, InstrumentConfigCommon>,
    time: InstrumentConfigBuilder<Histogram, HistogramConfig<f64>>,
}

impl CallMetricBuilder {
    /// Set the name of the metric we are tracking.
    pub fn name(mut self, name: impl AsRef<str>) -> Self {
        let name = name.as_ref();
        self.call = self
            .call
            .name(format!("{name}_attempt"))
            .description("number of times `method` has been called");
        self.success = self
            .success
            .name(format!("{name}_success"))
            .description("number of times `method` has completed in success");
        self.error = self
            .error
            .name(format!("{name}_error"))
            .description("number of times `method` has completed in error");
        self.time = self
            .time
            .name(format!("{name}_time"))
            .description("latencies for calls of `method`")
            .buckets(HistogramCommonBucket::Time);
        self
    }

    pub fn attrs<K: Into<Key>>(mut self, attrs: impl IntoIterator<Item = K>) -> Self {
        let attrs: Vec<_> = attrs.into_iter().map(Into::into).collect();
        self.call = self.call.attrs(attrs.clone());
        self.success = self.success.attrs(attrs.clone());
        self.error = self.error.attrs(attrs.clone());
        self.time = self.time.attrs(attrs);
        self
    }

    pub fn try_build(self, meter: &Meter) -> Result<CallMetric, InstrumentBuildError> {
        Ok(CallMetric {
            call: self.call.try_build(meter)?,
            success: self.success.try_build(meter)?,
            error: self.error.try_build(meter)?,
            time: self.time.try_build(meter)?,
        })
    }

    pub fn build(self, meter: &Meter) -> CallMetric {
        self.try_build(meter).unwrap()
    }
}

impl InstrumentBuild for CallMetric {
    type Config = CallMetricBuilder;

    fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
        Ok(Self {
            call: config.call.try_build(meter)?,
            success: config.success.try_build(meter)?,
            error: config.error.try_build(meter)?,
            time: config.time.try_build(meter)?,
        })
    }
}

impl MetricBinder for CallMetric {
    type Bound = BoundCallMetric;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes: Vec<_> = attributes.into_iter().map(Into::into).collect();
        Ok(BoundCallMetric {
            call: self.call.try_bind(attributes.clone())?,
            success: self.success.try_bind(attributes.clone())?,
            error: self.error.try_bind(attributes.clone())?,
            time: self.time.try_bind(attributes)?,
        })
    }
}

pub struct BoundCallMetric {
    call: BoundUIntCounter,
    success: BoundUIntCounter,
    error: BoundUIntCounter,
    time: BoundHistogram,
}

impl BoundCallMetric {
    pub fn enter(&self) -> CallMetricGuard<'_> {
        self.call.add(1);
        CallMetricGuard {
            on: Some(self),
            start_time: Instant::now(),
        }
    }
}

#[must_use = "returned guard should have `succeed`, `error`, or `exit` called on it later"]
pub struct CallMetricGuard<'a> {
    on: Option<&'a BoundCallMetric>,
    start_time: Instant,
}

impl CallMetricGuard<'_> {
    #[inline]
    pub fn succeed(self) {
        self.exit(true);
    }

    /// Succeed with the given `value`, returning it in an `Ok(value)`.
    #[inline]
    pub fn succeed_with<T, E>(self, value: T) -> Result<T, E> {
        self.exit_with(Ok(value))
    }

    #[inline]
    pub fn error(self) {
        self.exit(false);
    }

    /// Error with the given `error`, returning it in an `Err(error)`.
    #[inline]
    pub fn error_with<T, E>(self, error: E) -> Result<T, E> {
        self.exit_with(Err(error))
    }

    /// Complete in either success or error.
    #[inline]
    pub fn exit(mut self, success: bool) {
        self._complete(success);
    }

    /// Complete with the given `value`, returning it.
    ///
    /// This is identical to calling `complete(r.is_ok())`, but can be useful in situations where you are returning the
    /// result.
    #[inline]
    pub fn exit_with<T, E>(mut self, value: Result<T, E>) -> Result<T, E> {
        self._complete(value.is_ok());
        value
    }

    fn _complete(&mut self, success: bool) {
        let Some(owner) = self.on.take() else {
            return;
        };

        let counter = if success { &owner.success } else { &owner.error };
        counter.add(1);
        let elapsed = Instant::now() - self.start_time;
        owner.time.record(elapsed.as_secs_f64());
    }
}

/// Assume failure on drop.
impl Drop for CallMetricGuard<'_> {
    fn drop(&mut self) {
        self._complete(false)
    }
}

// =====================================================================================================================
// TimerMetric

pub struct TimerMetric {
    time: InitializedHistogramF64,
}

impl TimerMetric {
    pub fn builder() -> TimerMetricBuilder {
        TimerMetricBuilder::default()
    }
}

#[derive(Default)]
pub struct TimerMetricBuilder {
    time: InstrumentConfigBuilder<InitializedHistogramF64, HistogramConfig<f64>>,
}

impl TimerMetricBuilder {
    pub fn name(mut self, name: impl AsRef<str>) -> Self {
        self.time = self
            .time
            .name(name.as_ref().to_string())
            .description("time taken to complete the operation")
            .buckets(HistogramCommonBucket::Time);
        self
    }

    pub fn attrs<K: Into<Key>>(mut self, attrs: impl IntoIterator<Item = K>) -> Self {
        let attrs: Vec<_> = attrs.into_iter().map(Into::into).collect();
        self.time = self.time.attrs(attrs);
        self
    }

    pub fn try_build(self, meter: &Meter) -> Result<TimerMetric, InstrumentBuildError> {
        Ok(TimerMetric {
            time: self.time.try_build(meter)?,
        })
    }

    pub fn build(self, meter: &Meter) -> TimerMetric {
        self.try_build(meter).unwrap()
    }
}

impl InstrumentBuild for TimerMetric {
    type Config = TimerMetricBuilder;

    fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
        Ok(Self {
            time: config.time.try_build(meter)?,
        })
    }
}

impl MetricBinder for TimerMetric {
    type Bound = BoundTimerMetric;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes: Vec<_> = attributes.into_iter().map(Into::into).collect();
        Ok(BoundTimerMetric {
            time: self.time.try_bind(attributes)?,
        })
    }
}

#[derive(Clone)]
pub struct BoundTimerMetric {
    time: BoundHistogram,
}

impl BoundTimerMetric {
    pub fn start(&self) -> TimerMetricGuard {
        TimerMetricGuard {
            on: Some(self.clone()),
            start_time: Instant::now(),
        }
    }
}

pub struct TimerMetricGuard {
    on: Option<BoundTimerMetric>,
    start_time: Instant,
}

impl TimerMetricGuard {
    pub fn stop_and_record(self) {
        let mut timer = self;
        timer.observe(true);
    }

    pub fn stop_and_discard(self) {
        let mut timer = self;
        timer.observe(false);
    }

    fn observe(&mut self, record: bool) {
        if let Some(owner) = self.on.take() {
            if record {
                let elapsed = Instant::now() - self.start_time;
                owner.time.record(elapsed.as_secs_f64());
            }
        }
    }
}

impl Drop for TimerMetricGuard {
    fn drop(&mut self) {
        self.observe(true);
    }
}

#[cfg(test)]
mod prefix_tests {
    use super::{apply_prefix_with, normalize_prefix};
    use std::borrow::Cow;

    #[test]
    fn normalize_strips_trailing_dots() {
        assert_eq!(normalize_prefix(Cow::Borrowed("myapp")), "myapp");
        assert_eq!(normalize_prefix(Cow::Borrowed("myapp.")), "myapp");
        assert_eq!(normalize_prefix(Cow::Borrowed("myapp..")), "myapp");
        assert_eq!(normalize_prefix(Cow::Borrowed("myapp...")), "myapp");
    }

    #[test]
    fn normalize_strips_trailing_dots_owned() {
        assert_eq!(normalize_prefix(Cow::Owned("myapp".to_string())), "myapp");
        assert_eq!(normalize_prefix(Cow::Owned("myapp.".to_string())), "myapp");
        assert_eq!(normalize_prefix(Cow::Owned("myapp..".to_string())), "myapp");
    }

    #[test]
    fn normalize_preserves_internal_dots() {
        assert_eq!(normalize_prefix(Cow::Borrowed("my.app")), "my.app");
        assert_eq!(normalize_prefix(Cow::Borrowed("my.app.")), "my.app");
        assert_eq!(normalize_prefix(Cow::Borrowed("a.b.c.d")), "a.b.c.d");
    }

    #[test]
    fn normalize_preserves_underscores_and_other_separators() {
        // Underscores, hyphens, and slashes are deliberately preserved — they're caller intent, not noise.
        assert_eq!(
            normalize_prefix(Cow::Borrowed("my_namespace_derp")),
            "my_namespace_derp"
        );
        assert_eq!(normalize_prefix(Cow::Borrowed("my-namespace")), "my-namespace");
        assert_eq!(normalize_prefix(Cow::Borrowed("my_namespace.")), "my_namespace");
    }

    #[test]
    fn normalize_empty_and_dot_only() {
        assert_eq!(normalize_prefix(Cow::Borrowed("")), "");
        assert_eq!(normalize_prefix(Cow::Borrowed(".")), "");
        assert_eq!(normalize_prefix(Cow::Borrowed("..")), "");
        assert_eq!(normalize_prefix(Cow::Owned("...".to_string())), "");
    }

    #[test]
    fn apply_joins_with_dot() {
        assert_eq!(
            apply_prefix_with("storage", Cow::Borrowed("requests")),
            "storage.requests"
        );
        assert_eq!(
            apply_prefix_with("myapp", Cow::Borrowed("sdk.requests")),
            "myapp.sdk.requests"
        );
    }

    #[test]
    fn apply_empty_prefix_returns_bare_name() {
        assert_eq!(apply_prefix_with("", Cow::Borrowed("requests")), "requests");
    }

    #[test]
    fn apply_empty_name_returns_empty() {
        // Empty names are passed through; we don't manufacture a leading dot.
        assert_eq!(apply_prefix_with("storage", Cow::Borrowed("")), "");
        assert_eq!(apply_prefix_with("", Cow::Borrowed("")), "");
    }

    #[test]
    fn apply_preserves_underscore_namespace() {
        assert_eq!(
            apply_prefix_with("my_namespace_derp", Cow::Borrowed("requests")),
            "my_namespace_derp.requests"
        );
    }
}
