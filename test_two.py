import zenoh, struct, time, threading
s = zenoh.open(zenoh.Config())
p0 = s.declare_publisher("sim/eth/frame/0/tx")
p1 = s.declare_publisher("sim/eth/frame/1/tx")
rx = []
def on_rx(sample): 
    print("Received!", sample.key_expr)
    rx.append(sample)
sub = s.declare_subscriber("sim/eth/frame/*/rx", on_rx)
time.sleep(1)
p0.put(struct.pack("<QI", 0, 0)) # adds 0 to known
time.sleep(0.5)
p1.put(struct.pack("<QI", 0, 0)) # adds 1 to known, routes to 0
time.sleep(0.5)
p0.put(struct.pack("<QI", 0, 0)) # routes to 1
time.sleep(0.5)
print(f"Received {len(rx)}")
