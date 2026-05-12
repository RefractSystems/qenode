pub mod processors;

use alloc::sync::Arc;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{runtime, trace as sdktrace, Resource};
use processors::VTimeProvider;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

extern crate alloc;

pub struct OTelGuard {
    logger_provider: opentelemetry_sdk::logs::LoggerProvider,
    _runtime: Option<tokio::runtime::Runtime>,
}

impl Drop for OTelGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
        let _ = self.logger_provider.shutdown();
    }
}

fn build_pipeline(
    tracer_provider: sdktrace::TracerProvider,
    logger_provider: opentelemetry_sdk::logs::LoggerProvider,
    runtime: Option<tokio::runtime::Runtime>,
) -> OTelGuard {
    global::set_tracer_provider(tracer_provider.clone());
    let tracer = opentelemetry::trace::TracerProvider::tracer(&tracer_provider, "virtmcu");
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    let filter = EnvFilter::from_default_env();
    let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    let logger =
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logger_provider);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(telemetry)
        .with(logger)
        .try_init();

    OTelGuard {
        logger_provider,
        _runtime: runtime,
    }
}

/// Initialize OTel telemetry for a long-lived binary (Batch processor, tokio runtime).
/// Returns a guard; drop it at process exit to flush pending spans.
/// Reads OTEL_EXPORTER_OTLP_ENDPOINT (default: http://otel-collector:4317).
pub fn init_telemetry(
    service_name: &'static str,
    vtime_provider: Arc<dyn VTimeProvider>,
) -> OTelGuard {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://otel-collector:4317".to_owned());

    let mut tracer_builder = sdktrace::TracerProvider::builder().with_resource(Resource::new(
        vec![KeyValue::new("service.name", service_name)],
    ));

    let mut logger_builder =
        opentelemetry_sdk::logs::LoggerProvider::builder().with_resource(Resource::new(vec![
            KeyValue::new("service.name", service_name),
        ]));

    match SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .build()
    {
        Ok(trace_exporter) => {
            let batch_processor =
                sdktrace::BatchSpanProcessor::builder(trace_exporter, runtime::Tokio).build();
            let vtime_processor = processors::VTimeSpanProcessor {
                inner: batch_processor,
                provider: Arc::clone(&vtime_provider),
            };
            tracer_builder = tracer_builder.with_span_processor(vtime_processor);
        }
        Err(e) => {
            log::warn!("virtmcu-observability: Failed to create OTLP trace exporter: {e}. Proceeding with local tracing only.");
        }
    }

    match opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
    {
        Ok(log_exporter) => {
            let batch_log_processor =
                opentelemetry_sdk::logs::BatchLogProcessor::builder(log_exporter, runtime::Tokio)
                    .build();
            let vtime_log_processor = processors::VTimeLogProcessor {
                inner: batch_log_processor,
                provider: Arc::clone(&vtime_provider),
            };
            logger_builder = logger_builder.with_log_processor(vtime_log_processor);
        }
        Err(e) => {
            log::warn!("virtmcu-observability: Failed to create OTLP log exporter: {e}. Proceeding with local logging only.");
        }
    }

    build_pipeline(tracer_builder.build(), logger_builder.build(), None)
}

/// Initialize OTel for a QEMU TCG plugin (Simple/sync processor, no tokio).
/// Called after qemu_plugin_install succeeds. Returns a guard.
pub fn init_plugin_telemetry(
    service_name: &'static str,
    vtime_provider: Arc<dyn VTimeProvider>,
) -> OTelGuard {
    // The plugin has no tokio runtime to spawn background tasks on.
    // We create a lightweight single-threaded runtime to allow the OTLP exporters
    // and Batch processors to function correctly.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime for telemetry");

    let _guard = rt.enter();

    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://otel-collector:4317".to_owned());

    let mut tracer_builder = sdktrace::TracerProvider::builder().with_resource(Resource::new(
        vec![KeyValue::new("service.name", service_name)],
    ));

    let mut logger_builder =
        opentelemetry_sdk::logs::LoggerProvider::builder().with_resource(Resource::new(vec![
            KeyValue::new("service.name", service_name),
        ]));

    match SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .build()
    {
        Ok(trace_exporter) => {
            let simple_processor = sdktrace::SimpleSpanProcessor::new(Box::new(trace_exporter));
            let vtime_processor = processors::VTimeSpanProcessor {
                inner: simple_processor,
                provider: Arc::clone(&vtime_provider),
            };
            tracer_builder = tracer_builder.with_span_processor(vtime_processor);
        }
        Err(e) => {
            log::warn!("virtmcu-observability: Failed to create OTLP trace exporter: {e}. Proceeding with local tracing only.");
        }
    }

    match opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
    {
        Ok(log_exporter) => {
            let batch_log_processor =
                opentelemetry_sdk::logs::BatchLogProcessor::builder(log_exporter, runtime::Tokio)
                    .build();
            let vtime_log_processor = processors::VTimeLogProcessor {
                inner: batch_log_processor,
                provider: Arc::clone(&vtime_provider),
            };
            logger_builder = logger_builder.with_log_processor(vtime_log_processor);
        }
        Err(e) => {
            log::warn!("virtmcu-observability: Failed to create OTLP log exporter: {e}. Proceeding with local logging only.");
        }
    }

    build_pipeline(tracer_builder.build(), logger_builder.build(), Some(rt))
}
