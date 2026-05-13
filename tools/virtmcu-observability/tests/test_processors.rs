use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use opentelemetry::trace::{Span as _, Tracer, TracerProvider as _};
use opentelemetry::Context;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::trace::SpanProcessor;
use std::sync::Mutex;
use virtmcu_observability::processors::{VTimeProvider, VTimeSpanProcessor};

extern crate alloc;

#[derive(Debug)]
struct MockVTimeProvider(AtomicU64);

impl VTimeProvider for MockVTimeProvider {
    fn current_vtime_ns(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
struct MockSpanProcessor {
    ended_spans: Arc<Mutex<Vec<SpanData>>>,
}

impl MockSpanProcessor {
    fn new() -> Self {
        Self {
            ended_spans: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl SpanProcessor for MockSpanProcessor {
    fn on_start(&self, _span: &mut opentelemetry_sdk::trace::Span, _cx: &Context) {}

    fn on_end(&self, span: SpanData) {
        self.ended_spans.lock().expect("Failed to lock").push(span);
    }

    fn force_flush(&self) -> opentelemetry::trace::TraceResult<()> {
        Ok(())
    }

    fn shutdown(&self) -> opentelemetry::trace::TraceResult<()> {
        Ok(())
    }
}

#[test]
fn test_vtime_span_processor_injects_attributes() {
    let provider = Arc::new(MockVTimeProvider(AtomicU64::new(12345)));
    let mock_inner = MockSpanProcessor::new();
    let ended_spans = Arc::clone(&mock_inner.ended_spans);

    let processor = VTimeSpanProcessor {
        inner: mock_inner,
        provider: Arc::<MockVTimeProvider>::clone(&provider) as Arc<dyn VTimeProvider>,
    };

    let tracer_provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_span_processor(processor)
        .build();

    let tracer = tracer_provider.tracer("test");

    let mut span = tracer.start("test_span");
    // Advance virtual time
    provider.0.store(12350, Ordering::Relaxed);
    span.end();

    let ended = ended_spans.lock().expect("Failed to lock ended_spans");
    assert_eq!(ended.len(), 1);
    let attrs = &ended[0].attributes;

    let mut found_start = false;
    let mut found_end = false;
    for kv in attrs {
        if kv.key.as_str() == "vtime_ns" {
            if let opentelemetry::Value::I64(v) = kv.value {
                assert_eq!(v, 12345);
                found_start = true;
            }
        }
        if kv.key.as_str() == "vtime_ns_end" {
            if let opentelemetry::Value::I64(v) = kv.value {
                assert_eq!(v, 12350);
                found_end = true;
            }
        }
    }
    assert!(found_start, "vtime_ns attribute not found or invalid");
    assert!(found_end, "vtime_ns_end attribute not found or invalid");
}
