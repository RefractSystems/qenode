with open("hw/rust/virtmcu-api/tests/firmware_studio_mock.rs", "r") as f:
    text = f.read()

text = text.replace("assert_eq!(mmio.vtime_ns, 0);", "assert_eq!({ mmio.vtime_ns }, 0);")
text = text.replace("assert_eq!(sysc.type_, 0);", "assert_eq!({ sysc.type_ }, 0);")
text = text.replace("assert_eq!(clk_adv.delta_ns, 0);", "assert_eq!({ clk_adv.delta_ns }, 0);")
text = text.replace("assert_eq!(clk_rdy.error_code, 0);", "assert_eq!({ clk_rdy.error_code }, 0);")

with open("hw/rust/virtmcu-api/tests/firmware_studio_mock.rs", "w") as f:
    f.write(text)
