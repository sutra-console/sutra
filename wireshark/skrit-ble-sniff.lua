-- skrit-ble-sniff.lua — Wireshark dissector for sutra-extcap USER0 packets.
-- ===========================================================================
-- sutra-extcap captures a Duta's ble-sniff DATA stream as classic pcap with
-- LINKTYPE_USER0 (147). Each USER0 packet is exactly one ble-sniff record
-- (PROTOCOL.md "BLE sniffer"):
--
--   ts_ms(4 LE) · channel(1) · rssi(1, magnitude) · access-address(4 LE) ·
--   pdu_len(1) · pdu…
--
-- where `pdu` is the on-air BLE advertising PDU (2-byte header + payload),
-- already de-whitened and CRC-checked by the radio. This script turns the raw
-- USER0 hex into real fields: channel, RSSI, access address, PDU type, the
-- advertising address, and the parsed AD structures (name, flags, mfr data…).
--
-- Install: copy into Wireshark's Personal Lua Plugins folder
--   (Help ▸ About ▸ Folders ▸ "Personal Lua Plugins";
--    %APPDATA%\Wireshark\plugins on Windows), then restart Wireshark / Ctrl+Shift+L.
-- It auto-binds to USER0, so no DLT_USER preference setup is needed.
--
-- Scope note: advertising PDUs only (what the sniffer captures today). This is
-- a stopgap until sutra-extcap emits real LINKTYPE_NORDIC_BLE — see EXTCAP.md.
-- ===========================================================================

local p_sniff = Proto("skrit_blesniff", "Skrit BLE Sniffer")

local pdu_types = {
  [0] = "ADV_IND", [1] = "ADV_DIRECT_IND", [2] = "ADV_NONCONN_IND",
  [3] = "SCAN_REQ", [4] = "SCAN_RSP", [5] = "CONNECT_IND",
  [6] = "ADV_SCAN_IND", [7] = "ADV_EXT_IND",
}
local ad_types = {
  [0x01] = "Flags", [0x02] = "Incomplete 16-bit UUIDs",
  [0x03] = "Complete 16-bit UUIDs", [0x06] = "Incomplete 128-bit UUIDs",
  [0x07] = "Complete 128-bit UUIDs", [0x08] = "Shortened Local Name",
  [0x09] = "Complete Local Name", [0x0A] = "Tx Power Level",
  [0x16] = "Service Data (16-bit)", [0x19] = "Appearance",
  [0xFF] = "Manufacturer Specific",
}

local f = p_sniff.fields
f.ts      = ProtoField.uint32("skrit_blesniff.ts_ms",   "Timestamp (device ms)", base.DEC)
f.channel = ProtoField.uint8 ("skrit_blesniff.channel", "Channel",        base.DEC)
f.rssi    = ProtoField.int32 ("skrit_blesniff.rssi",    "RSSI (dBm)",     base.DEC)
f.aa      = ProtoField.uint32("skrit_blesniff.aa",      "Access Address", base.HEX)
f.pdu_len = ProtoField.uint8 ("skrit_blesniff.pdu_len", "PDU Length",     base.DEC)
f.ptype   = ProtoField.uint8 ("skrit_blesniff.pdu_type", "PDU Type", base.HEX, pdu_types, 0x0F)
f.txadd   = ProtoField.uint8 ("skrit_blesniff.txadd", "TxAdd", base.DEC, {[0]="Public",[1]="Random"}, 0x40)
f.rxadd   = ProtoField.uint8 ("skrit_blesniff.rxadd", "RxAdd", base.DEC, {[0]="Public",[1]="Random"}, 0x80)
f.advaddr = ProtoField.bytes ("skrit_blesniff.adv_addr", "Advertising Address")
f.ad_len  = ProtoField.uint8 ("skrit_blesniff.ad.len",  "Length", base.DEC)
f.ad_type = ProtoField.uint8 ("skrit_blesniff.ad.type", "Type",   base.HEX, ad_types)
f.ad_val  = ProtoField.bytes ("skrit_blesniff.ad.value", "Value")
f.name    = ProtoField.string("skrit_blesniff.name",   "Local Name")
f.payload = ProtoField.bytes ("skrit_blesniff.payload", "PDU Payload")

-- Render 6 little-endian address bytes as a colon-separated MAC (MSB first).
local function addr_str(range)
  local b = range:bytes()
  local parts = {}
  for i = 5, 0, -1 do parts[#parts + 1] = string.format("%02x", b:get_index(i)) end
  return table.concat(parts, ":")
end

-- Parse the AD-structure list (len · type · value)… of an advertising payload.
local function dissect_ad(tree, pdu, off, plen)
  local name
  while off + 1 < plen do
    local adlen = pdu(off, 1):uint()
    if adlen == 0 or off + 1 + adlen > plen then break end
    local adtype = pdu(off + 1, 1):uint()
    local label = ad_types[adtype] or string.format("Type 0x%02x", adtype)
    local adtree = tree:add(p_sniff, pdu(off, adlen + 1), "AD Structure: " .. label)
    adtree:add(f.ad_len, pdu(off, 1))
    adtree:add(f.ad_type, pdu(off + 1, 1))
    if adlen > 1 then
      adtree:add(f.ad_val, pdu(off + 2, adlen - 1))
      if adtype == 0x08 or adtype == 0x09 then
        name = pdu(off + 2, adlen - 1):string()
        adtree:add(f.name, pdu(off + 2, adlen - 1))
      end
    end
    off = off + 1 + adlen
  end
  return name
end

function p_sniff.dissector(tvb, pinfo, tree)
  local len = tvb:len()
  if len < 11 then return 0 end
  pinfo.cols.protocol = "BLE-Sniff"

  local st = tree:add(p_sniff, tvb(), "Skrit BLE Sniffer record")
  st:add_le(f.ts, tvb(0, 4))
  local ch = tvb(4, 1):uint()
  st:add(f.channel, tvb(4, 1))
  local rssi = -(tvb(5, 1):uint()) -- stored as magnitude; real value is negative dBm
  st:add(f.rssi, tvb(5, 1), rssi)
  st:add_le(f.aa, tvb(6, 4))
  local plen = tvb(10, 1):uint()
  st:add(f.pdu_len, tvb(10, 1))

  if plen < 2 or len < 11 + plen then
    pinfo.cols.info = string.format("ch%d  %d dBm  (truncated)", ch, rssi)
    return
  end

  local pdu = tvb(11, plen)
  local b0 = pdu(0, 1):uint()
  local ptype = b0 % 16 -- low nibble = PDU type (no bit lib needed)
  local tname = pdu_types[ptype] or string.format("0x%02x", ptype)
  local ptree = st:add(p_sniff, pdu, "Advertising PDU: " .. tname)
  ptree:add(f.ptype, pdu(0, 1))
  ptree:add(f.txadd, pdu(0, 1))
  ptree:add(f.rxadd, pdu(0, 1))

  local name, src, dst
  if ptype == 0 or ptype == 2 or ptype == 6 or ptype == 4 then
    -- ADV_IND / ADV_NONCONN_IND / ADV_SCAN_IND / SCAN_RSP: AdvA(6) + AD list
    src = addr_str(pdu(2, 6))
    dst = "Broadcast"
    ptree:add(f.advaddr, pdu(2, 6)):set_text("Advertising Address: " .. src)
    name = dissect_ad(st, pdu, 8, plen)
  elseif ptype == 1 then
    -- ADV_DIRECT_IND: AdvA(6, source) -> TargetA(6, dest)
    src = addr_str(pdu(2, 6))
    ptree:add(f.advaddr, pdu(2, 6)):set_text("Advertising Address: " .. src)
    if plen >= 14 then
      dst = addr_str(pdu(8, 6))
      ptree:add(f.payload, pdu(8, 6)):set_text("Target Address: " .. dst)
    end
  elseif ptype == 3 or ptype == 5 then
    -- SCAN_REQ / CONNECT_IND: ScanA/InitA(6, source) -> AdvA(6, dest)
    if plen >= 14 then
      src = addr_str(pdu(2, 6))
      dst = addr_str(pdu(8, 6))
      ptree:add(f.payload, pdu(2, 6)):set_text("Scanner/Initiator Address: " .. src)
      ptree:add(f.advaddr, pdu(8, 6)):set_text("Advertising Address: " .. dst)
    else
      st:add(f.payload, pdu(2, plen - 2))
    end
  else
    -- ext / unknown: show the raw payload (not parsed in v1)
    st:add(f.payload, pdu(2, plen - 2))
  end

  -- Fill Wireshark's Source / Destination columns from the BLE addresses.
  if src then pinfo.cols.src = src end
  if dst then pinfo.cols.dst = dst end

  pinfo.cols.info = string.format("ch%d  %d dBm  %s%s", ch, rssi, tname,
    name and ("  — " .. name) or "")
end

-- Bind to USER0 (LINKTYPE_USER0 = 147). Registering on wtap_encap means the
-- sutra-extcap pcap is dissected with zero per-capture DLT_USER configuration.
local wtap_encap = DissectorTable.get("wtap_encap")
wtap_encap:add(wtap.USER0, p_sniff)
