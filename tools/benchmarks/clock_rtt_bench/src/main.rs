use clap::Parser;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use virtmcu_wire::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt};

#[derive(Parser, Debug)]
#[command(author, version, about = "Simulation Frequency Ceiling Benchmark", long_about = None)]
struct Args {
    /// Number of quanta to simulate
    #[arg(long, default_value_t = 10_000)]
    count: usize,

    /// Payload size (actually ignored since ClockAdvanceReq is fixed size, but kept for compatibility conceptually)
    #[arg(long, default_value_t = 24)]
    size: usize,

    /// Use Unix socket transport instead of Zenoh
    #[arg(long, default_value_t = false)]
    unix: bool,

    /// Router connection string. For Zenoh, e.g., tcp/localhost:7447. For Unix, the socket path, e.g., /tmp/virtmcu_bench.sock
    #[arg(long, default_value = "tcp/localhost:7447")]
    router: String,

    /// Topic to use (for Zenoh)
    #[arg(long, default_value = "sim/clock/advance/0")]
    topic: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let count = args.count;

    tracing::info!("Starting benchmark: {} gets", count);

    let mut rtts: Vec<f64> = Vec::with_capacity(count);

    if args.unix {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        let path = PathBuf::from(&args.router);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        tracing::info!("Listening on Unix socket: {:?}", path);

        let (mut stream, _addr) = listener.accept().await?;
        tracing::info!("Client connected.");

        let mut current_vtime = 0u64;

        for i in 0..count {
            let req = ClockAdvanceReq::new(10_000_000, current_vtime + 10_000_000, i as u64);
            let req_bytes = req.pack();

            let t0 = Instant::now();
            stream.write_all(req_bytes).await?;

            let mut resp_buf = [0u8; 24]; // ClockReadyResp is 24 bytes
            stream.read_exact(&mut resp_buf).await?;

            let t1 = Instant::now();
            rtts.push(t1.duration_since(t0).as_secs_f64() * 1000.0);

            if let Some(resp) = ClockReadyResp::unpack_slice(&resp_buf) {
                current_vtime = resp.current_vtime_ns();
            }
        }
    } else {
        use zenoh::{Config, Wait};

        let mut config = Config::default();
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", args.router))
            .map_err(|e| e.to_string())?;

        let session = zenoh::open(config).wait().map_err(|e| e.to_string())?;
        tracing::info!("Connected to Zenoh router.");

        let mut current_vtime = 0u64;

        for i in 0..count {
            let req = ClockAdvanceReq::new(10_000_000, current_vtime + 10_000_000, i as u64);
            let req_bytes = req.pack().to_vec();

            let t0 = Instant::now();

            let replies = session
                .get(&args.topic)
                .payload(req_bytes)
                .timeout(Duration::from_secs(5))
                .wait()
                .map_err(|e| e.to_string())?;

            let mut got_reply = false;
            while let Ok(reply) = replies.recv() {
                got_reply = true;
                if let Ok(sample) = reply.result() {
                    if let Some(resp) = ClockReadyResp::unpack_slice(&sample.payload().to_bytes()) {
                        current_vtime = resp.current_vtime_ns();
                    }
                }
            }

            let t1 = Instant::now();
            if !got_reply {
                tracing::error!("No reply received!");
            }
            rtts.push(t1.duration_since(t0).as_secs_f64() * 1000.0);
        }
    }

    if !rtts.is_empty() {
        let n = rtts.len() as f64;
        let sum: f64 = rtts.iter().sum();
        let mean = sum / n;

        let mut sorted = rtts.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min = sorted[0];
        let max = sorted[sorted.len() - 1];
        let median = sorted[sorted.len() / 2];
        let p99 = sorted[(sorted.len() as f64 * 0.99) as usize];

        tracing::info!("Count: {}", sorted.len());
        tracing::info!("Mean:   {:.3} ms", mean);
        tracing::info!("Min:    {:.3} ms", min);
        tracing::info!("Max:    {:.3} ms", max);
        tracing::info!("Median: {:.3} ms", median);
        tracing::info!("P99:    {:.3} ms", p99);
    }

    Ok(())
}
