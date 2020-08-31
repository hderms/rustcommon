// Copyright 2019-2020 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

use crate::*;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ChannelStatistic {
    name: String,
    description: Option<String>,
    source: Source,
    unit: Option<String>,
}

impl ChannelStatistic {
    fn new(statistic: &dyn Statistic) -> Self {
        Self {
            name: statistic.name().to_string(),
            description: statistic.description().map(|v| v.to_string()),
            source: statistic.source(),
            unit: statistic.unit().map(|v| v.to_string()),
        }
    }
}

impl Statistic for ChannelStatistic {
    fn name(&self) -> &str {
        &self.name
    }

    fn source(&self) -> Source {
        self.source
    }

    fn description(&self) -> Option<&str> {
        self.description.as_ref().map(|v| v.as_ref())
    }

    fn unit(&self) -> Option<&str> {
        self.unit.as_ref().map(|v| v.as_ref())
    }
}

/// A channel tracks measurements that are taken from the same datasource. For
/// example, you might use a channel to track requests and another for CPU
/// utilization.
pub struct Channel<T>
where
    T: Unsigned + SaturatingArithmetic + Default + FetchCompareStore,
    <T as Atomic>::Primitive: Default + PartialEq + Copy + From<u8>,
    u64: From<<T as Atomic>::Primitive>,
{
    statistic: ChannelStatistic,
    source: Source,
    reading: AtomicU64,
    histogram: Option<Histogram<T>>,
    last_write: AtomicU64,
    latched: bool,
    max: Point,
    min: Point,
    outputs: Arc<Mutex<HashSet<Output>>>,
    has_data: AtomicBool,
}

impl<T: 'static> PartialEq for Channel<T>
where
    T: Unsigned + SaturatingArithmetic + Default + FetchCompareStore,
    <T as Atomic>::Primitive: Default + PartialEq + Copy + From<u8>,
    u64: From<<T as Atomic>::Primitive>,
{
    fn eq(&self, other: &Channel<T>) -> bool {
        self.statistic.name() == other.statistic.name()
    }
}

impl<T: 'static> Eq for Channel<T>
where
    T: Unsigned + SaturatingArithmetic + Default + FetchCompareStore,
    <T as Atomic>::Primitive: Default + PartialEq + Copy + From<u8>,
    u64: From<<T as Atomic>::Primitive>,
{
}

impl<T: 'static> Channel<T>
where
    T: Unsigned + SaturatingArithmetic + Default + FetchCompareStore,
    <T as Atomic>::Primitive: Default + PartialEq + Copy + From<u8>,
    u64: From<<T as Atomic>::Primitive>,
{
    /// Create a new channel with a given name, source, and an optional
    /// histogram which can be used to generate percentile metrics
    pub fn new(statistic: &dyn Statistic, summary: Option<Summary>) -> Self {
        let histogram = if let Some(summary) = summary {
            summary.build_histogram::<T>()
        } else {
            None
        };
        Self {
            statistic: ChannelStatistic::new(statistic),
            source: statistic.source(),
            reading: AtomicU64::default(),
            histogram,
            last_write: AtomicU64::new(time::precise_time_ns()),
            latched: true,
            max: Point::new(0, 0),
            min: Point::new(0, 0),
            outputs: Arc::new(Mutex::new(HashSet::new())),
            has_data: AtomicBool::new(false),
        }
    }

    // for Counter measurements:
    // reading tracks value
    // histogram tracks rate of change
    pub fn record_counter(&self, time: u64, value: u64) {
        if self.source == Source::Counter {
            if self.has_data.load(Ordering::Relaxed) {
                let last_write = self.last_write.load(Ordering::Relaxed);
                // calculate the difference between consecutive readings and the rate
                let delta_value = value.wrapping_sub(self.reading.load(Ordering::Relaxed));
                let delta_time = time.wrapping_sub(last_write);
                let rate = ((delta_value as f64 / delta_time as f64) * 1_000_000_000.0) as u64;
                self.reading.fetch_add(delta_value, Ordering::Relaxed);
                if let Some(ref histogram) = self.histogram {
                    histogram.increment(rate, <T as Atomic>::Primitive::from(1_u8));
                }
                // track the point of max rate
                if self.max.time() > 0 {
                    if rate > self.max.value() {
                        self.max.set(rate, time);
                    }
                } else {
                    self.max.set(rate, time);
                }
                // track the point of min rate
                if self.min.time() > 0 {
                    if rate < self.min.value() {
                        self.min.set(rate, time);
                    }
                } else {
                    self.min.set(rate, time);
                }
            } else {
                self.reading.store(value, Ordering::Relaxed);
                self.has_data.store(true, Ordering::Relaxed);
            }
            self.last_write.store(time, Ordering::Relaxed);
        }
    }

    // for Delta measurements:
    // reading is sum of values
    // histogram tracks rate of change
    pub fn record_delta(&self, time: u64, value: u64) {
        if self.source == Source::Counter {
            if self.has_data.load(Ordering::SeqCst) {
                // calculate the rate
                let last_write = self.last_write.load(Ordering::Relaxed);
                let delta_time = time - last_write;
                let rate = (value as f64 * (1_000_000_000.0 / delta_time as f64)) as u64;
                self.reading.fetch_add(value, Ordering::Relaxed);
                if let Some(ref histogram) = self.histogram {
                    histogram.increment(rate, <T as Atomic>::Primitive::from(1_u8));
                }
                // track the point of max rate
                if self.max.time() > 0 {
                    if rate > self.max.value() {
                        self.max.set(rate, time);
                    }
                } else {
                    self.max.set(rate, time);
                }
                // track the point of min rate
                if self.min.time() > 0 {
                    if rate < self.min.value() {
                        self.min.set(rate, time);
                    }
                } else {
                    self.min.set(rate, time);
                }
            } else {
                self.reading.store(value, Ordering::Relaxed);
                self.has_data.store(true, Ordering::SeqCst);
            }
            self.last_write.store(time, Ordering::Relaxed);
        }
    }

    // for Distribution measurements:
    // reading tracks sum of all counts
    // histogram tracks values
    pub fn record_distribution(&self, time: u64, value: u64, count: <T as Atomic>::Primitive) {
        if self.source == Source::Distribution {
            self.reading.fetch_add(u64::from(count), Ordering::Relaxed);
            if let Some(ref histogram) = self.histogram {
                histogram.increment(value, count);
            }
            self.last_write.store(time, Ordering::Relaxed);
        }
    }

    // for Gauge measurements:
    // reading tracks latest reading
    // histogram tracks readings
    // max tracks largest reading
    // min tracks smallest reading
    pub fn record_gauge(&self, time: u64, value: u64) {
        if self.source == Source::Gauge {
            self.reading.store(value, Ordering::Relaxed);
            if let Some(ref histogram) = self.histogram {
                histogram.increment(value, <T as Atomic>::Primitive::from(1_u8));
            }
            // track the point of max gauge reading
            if self.max.time() > 0 {
                if value > self.max.value() {
                    self.max.set(value, time);
                }
            } else {
                self.max.set(value, time);
            }
            // track the point of min rate
            if self.min.time() > 0 {
                if value < self.min.value() {
                    self.min.set(value, time);
                }
            } else {
                self.min.set(value, time);
            }
            self.last_write.store(time, Ordering::Relaxed);
        }
    }

    // for Increment measurements:
    // reading tracks sum of all increments
    // histogram tracks magnitude of increments
    pub fn record_increment(&self, time: u64, count: <T as Atomic>::Primitive) {
        if self.source == Source::Counter {
            self.reading.fetch_add(u64::from(count), Ordering::Relaxed);
            if let Some(ref histogram) = self.histogram {
                histogram.increment(u64::from(count), <T as Atomic>::Primitive::from(1_u8));
            }
            self.last_write.store(time, Ordering::Relaxed);
        }
    }

    // for TimeInterval measurements, we increment the histogram with duration of event
    // reading tracks number of events
    pub fn record_time_interval(&self, start: u64, stop: u64) {
        if self.source == Source::TimeInterval {
            self.reading.fetch_add(1, Ordering::Relaxed);
            let duration = stop.wrapping_sub(start);
            if let Some(ref histogram) = self.histogram {
                histogram.increment(duration, <T as Atomic>::Primitive::from(1_u8));
            }
            // track point of largest interval
            if self.max.time() > 0 {
                if duration > self.max.value() {
                    self.max.set(duration, start);
                }
            } else {
                self.max.set(duration, start);
            }
            // track point of smallest interval
            if self.min.time() > 0 {
                if duration < self.min.value() {
                    self.min.set(duration, start);
                }
            } else {
                self.min.set(duration, start);
            }
        }
    }

    /// Get the reading (counter or gauge) from the `Channel`
    pub fn reading(&self) -> u64 {
        self.reading.load(Ordering::Relaxed)
    }

    /// Calculate a percentile from the histogram, returns `None` if there is no
    /// histogram for the `Channel`
    pub fn percentile(&self, percentile: f64) -> Result<u64, MetricsError> {
        if let Some(ref histogram) = self.histogram {
            histogram
                .percentile(percentile)
                .map_err(|_| MetricsError::EmptyChannel)
        } else {
            Err(MetricsError::NoSummary)
        }
    }

    /// Register an `Output` for exposition
    pub fn add_output(&self, output: Output) {
        let mut outputs = self.outputs.lock().unwrap();
        outputs.insert(output);
    }

    /// Remove an `Output` from exposition
    pub fn delete_output(&self, output: Output) {
        let mut outputs = self.outputs.lock().unwrap();
        outputs.remove(&output);
    }

    /// Resets any latched aggregators, `Histograms` may be latched or windowed.
    /// Min and max value-time tracking are currently always latched and need to
    /// be reset using this function.
    pub fn latch(&self) {
        if self.latched {
            if let Some(ref histogram) = self.histogram {
                histogram.clear();
            }
        }
        self.max.set(0, 0);
        self.min.set(0, 0);
    }

    /// Zeros out all stored data for the `Channel`
    pub fn zero(&self) {
        self.has_data.store(false, Ordering::SeqCst);
        self.last_write
            .store(time::precise_time_ns(), Ordering::Relaxed);
        self.reading.store(0, Ordering::Relaxed);
        if let Some(ref histogram) = self.histogram {
            histogram.clear();
        }
        self.max.set(0, 0);
        self.min.set(0, 0);
    }

    /// Calculates the total set of `Readings` that are produced based on the
    /// `Outputs` which have been added for the `Channel`
    pub fn readings(&self) -> Vec<Reading> {
        let mut result = Vec::new();
        let outputs = self.outputs.lock().unwrap();
        for output in &*outputs {
            match output {
                Output::Reading => {
                    result.push(Reading::new(
                        self.statistic.name().to_string(),
                        output.clone(),
                        self.reading(),
                    ));
                }
                Output::MaxPointTime => {
                    if self.max.time() > 0 {
                        result.push(Reading::new(
                            self.statistic.name().to_string(),
                            output.clone(),
                            self.max.time(),
                        ));
                    }
                }
                Output::MinPointTime => {
                    if self.max.time() > 0 {
                        result.push(Reading::new(
                            self.statistic.name().to_string(),
                            output.clone(),
                            self.min.time(),
                        ));
                    }
                }
                Output::Percentile(percentile) => {
                    if let Ok(value) = self.percentile(percentile.as_f64()) {
                        result.push(Reading::new(
                            self.statistic.name().to_string(),
                            output.clone(),
                            value,
                        ));
                    }
                }
            }
        }
        result
    }

    /// Calculates and returns the `Output`s with their values as a `HashMap`
    pub fn hash_map(&self) -> HashMap<Output, u64> {
        let mut result = HashMap::new();
        let outputs = self.outputs.lock().unwrap();
        for output in &*outputs {
            match output {
                Output::Reading => {
                    result.insert(output.clone(), self.reading());
                }
                Output::MaxPointTime => {
                    if self.max.time() > 0 {
                        result.insert(output.clone(), self.max.time());
                    }
                }
                Output::MinPointTime => {
                    if self.max.time() > 0 {
                        result.insert(output.clone(), self.min.time());
                    }
                }
                Output::Percentile(percentile) => {
                    if let Ok(value) = self.percentile(percentile.as_f64()) {
                        result.insert(output.clone(), value);
                    }
                }
            }
        }
        result
    }
}
