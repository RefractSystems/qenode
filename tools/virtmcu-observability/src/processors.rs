extern crate alloc;

use alloc::sync::Arc;
use core::fmt::Debug;
use opentelemetry::logs::LogRecord;
use opentelemetry::trace::Span;
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::logs::LogProcessor;
use opentelemetry_sdk::trace::SpanProcessor;

pub trait VTimeProvider: Send + Sync + Debug {
    fn current_vtime_ns(&self) -> u64;
}

#[derive(Debug)]
pub struct VTimeSpanProcessor<T: SpanProcessor> {
    pub inner: T,
    pub provider: Arc<dyn VTimeProvider>,
}

impl<T: SpanProcessor> SpanProcessor for VTimeSpanProcessor<T> {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &Context) {
        let vtime = self.provider.current_vtime_ns();
        if vtime > 0 {
            span.set_attribute(KeyValue::new("vtime_ns", vtime as i64));
        }
        self.inner.on_start(span, cx);
    }

    fn on_end(&self, mut span: SpanData) {
        let vtime = self.provider.current_vtime_ns();
        if vtime > 0 {
            span.attributes
                .push(KeyValue::new("vtime_ns_end", vtime as i64));
        }
        self.inner.on_end(span);
    }

    fn force_flush(&self) -> opentelemetry::trace::TraceResult<()> {
        self.inner.force_flush()
    }

    fn shutdown(&self) -> opentelemetry::trace::TraceResult<()> {
        self.inner.shutdown()
    }
}

#[derive(Debug)]
pub struct VTimeLogProcessor<T: LogProcessor> {
    pub inner: T,
    pub provider: Arc<dyn VTimeProvider>,
}

impl<T: LogProcessor> LogProcessor for VTimeLogProcessor<T> {
    fn emit(
        &self,
        record: &mut opentelemetry_sdk::logs::LogRecord,
        instrumentation: &opentelemetry::InstrumentationScope,
    ) {
        let vtime = self.provider.current_vtime_ns();
        if vtime > 0 {
            record.add_attribute("vtime_ns", opentelemetry::logs::AnyValue::Int(vtime as i64));
        }
        self.inner.emit(record, instrumentation);
    }

    fn force_flush(&self) -> opentelemetry_sdk::logs::LogResult<()> {
        self.inner.force_flush()
    }

    fn shutdown(&self) -> opentelemetry_sdk::logs::LogResult<()> {
        self.inner.shutdown()
    }
}
