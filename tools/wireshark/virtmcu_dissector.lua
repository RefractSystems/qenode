-- virtmcu_dissector.lua
-- Wireshark Dissector for VirtMCU Custom Link Type (DLT_USER0 / 147)

local virtmcu_proto = Proto("virtmcu", "VirtMCU Protocol")

-- Fields
local f_src_node = ProtoField.uint32("virtmcu.src_node", "Source Node ID", base.DEC)
local f_dst_node = ProtoField.uint32("virtmcu.dst_node", "Destination Node ID", base.DEC)
local f_protocol = ProtoField.uint16("virtmcu.protocol", "Protocol", base.DEC, {
    [1] = "Ethernet",
    [2] = "UART",
    [3] = "IEEE 802.15.4",
    [4] = "CAN-FD",
    [5] = "FlexRay",
    [6] = "LIN",
    [7] = "SPI",
    [8] = "RF-HCI",
    [255] = "Test Infrastructure"
})
local f_payload = ProtoField.bytes("virtmcu.payload", "Payload")

-- Test Infra Fields
local f_test_topic_len = ProtoField.uint16("virtmcu.test.topic_len", "Topic Length", base.DEC)
local f_test_topic = ProtoField.string("virtmcu.test.topic", "Topic")
local f_test_dir_len = ProtoField.uint16("virtmcu.test.dir_len", "Direction Length", base.DEC)
local f_test_dir = ProtoField.string("virtmcu.test.direction", "Direction")

virtmcu_proto.fields = {
    f_src_node, f_dst_node, f_protocol, f_payload,
    f_test_topic_len, f_test_topic, f_test_dir_len, f_test_dir
}

function virtmcu_proto.dissector(buffer, pinfo, tree)
    pinfo.cols.protocol = "VirtMCU"
    local subtree = tree:add(virtmcu_proto, buffer(), "VirtMCU Protocol Data")

    subtree:add(f_src_node, buffer(0, 4))
    subtree:add(f_dst_node, buffer(4, 4))
    
    local protocol_id = buffer(8, 2):le_uint()
    subtree:add(f_protocol, buffer(8, 2))

    local payload_start = 10
    if protocol_id == 255 then
        -- Test Infrastructure has extra metadata
        local topic_len = buffer(10, 2):le_uint()
        subtree:add(f_test_topic_len, buffer(10, 2))
        subtree:add(f_test_topic, buffer(12, topic_len))
        
        local dir_start = 12 + topic_len
        local dir_len = buffer(dir_start, 2):le_uint()
        subtree:add(f_test_dir_len, buffer(dir_start, 2))
        subtree:add(f_test_dir, buffer(dir_start + 2, dir_len))
        
        payload_start = dir_start + 2 + dir_len
    end

    local payload_len = buffer:len() - payload_start
    if payload_len > 0 then
        subtree:add(f_payload, buffer(payload_start, payload_len))
        
        -- Delegate to sub-dissectors based on protocol
        if protocol_id == 1 then -- Ethernet
            local eth_dissector = Dissector.get("eth")
            if eth_dissector then
                eth_dissector:call(buffer(payload_start):tvb(), pinfo, tree)
            end
        elseif protocol_id == 4 then -- CAN-FD
            local can_dissector = Dissector.get("canfd")
            if can_dissector then
                can_dissector:call(buffer(payload_start):tvb(), pinfo, tree)
            end
        elseif protocol_id == 3 then -- IEEE 802.15.4
            local wpan_dissector = Dissector.get("wpan")
            if wpan_dissector then
                wpan_dissector:call(buffer(payload_start):tvb(), pinfo, tree)
            end
        end
    end
end

-- Register to DLT_USER0 (147)
local wtap_encap_table = DissectorTable.get("wtap_encap")
wtap_encap_table:add(147, virtmcu_proto)
