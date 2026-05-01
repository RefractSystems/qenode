use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use virtmcu_api::{
    FlatBufferStructExt, MmioReq, SyscMsg, VirtmcuHandshake, SYSC_MSG_RESP, VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <socket_path>", args[0]);
        std::process::exit(1);
    }

    let socket_path = &args[1];

    // Remove existing socket if it exists
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)?;
    println!("Stress adapter listening on {}...", socket_path);

    let (mut stream, _) = listener.accept()?;
    println!("Client connected, starting handshake...");

    // Read Handshake
    let mut hs_buf = [0u8; 8];
    stream.read_exact(&mut hs_buf)?;

    let _hs_in = VirtmcuHandshake::unpack_slice(&hs_buf).expect("Failed to unpack handshake");
    // Could check magic/version here, but we'll just echo our own

    let hs_out = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION);
    stream.write_all(hs_out.pack())?;

    println!("Handshake complete. Starting stress loop...");

    let mut req_buf = [0u8; 32]; // MmioReq size
    let mut count = 0u64;
    let start_time = std::time::Instant::now();

    while stream.read_exact(&mut req_buf).is_ok() {
        let req = MmioReq::unpack_slice(&req_buf).expect("Failed to unpack MmioReq");
        let resp = SyscMsg::new(SYSC_MSG_RESP, 0, req.data());
        if stream.write_all(resp.pack()).is_err() {
            break;
        }
        count += 1;
        if count.is_multiple_of(100_000) {
            let elapsed = start_time.elapsed().as_secs_f64();
            println!(
                "Processed {} requests ({:.2} req/s)",
                count,
                count as f64 / elapsed
            );
        }
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    println!(
        "Stress adapter exiting. Processed {} requests in {:.2}s ({:.2} req/s)",
        count,
        elapsed,
        count as f64 / elapsed
    );

    let _ = std::fs::remove_file(socket_path);
    Ok(())
}
