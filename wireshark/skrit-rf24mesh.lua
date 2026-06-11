-- skrit-rf24mesh.lua — Wireshark dissector for RF24Network / RF24Mesh
-- (TMRh20's nRF24L01+ mesh stack). Handles both the v1 (legacy) and v2 (current)
-- wire revisions via a protocol preference.
-- ===========================================================================
-- RF24Mesh layers on RF24Network, which layers on nRF24 Enhanced ShockBurst:
--
--   ESB frame  ->  RF24NetworkHeader (8 bytes)  ->  Mesh control / app payload
--
-- This dissector decodes the RF24Network header and the known Mesh/Network
-- control payloads. The 8-byte header is identical across v1 and v2:
--
--   from_node(2 LE) · to_node(2 LE) · id(2 LE) · type(1) · reserved(1)
--
-- Node addresses are OCTAL logical addresses (00 = master, 01..05 = its direct
-- children, 011..055 = their children, …) — shown in octal, the way the library
-- means them. Wireshark has no native RF24 support, so this is the dissector.
--
-- Wiring: registers on USER1 (LINKTYPE_USER0+1 = 148), the slot the future
-- `esb` extcap path will use, and also exports a named "rf24network" dissector
-- so an ESB base dissector can hand its payload up (see DECODERS.md sub-decoders).
--
-- ===========================================================================
-- VERSIONS — what differs between v1 and v2:
--   * The 8-byte header layout: IDENTICAL. (so most decoding is version-agnostic)
--   * A few system message-type CONSTANTS shifted (notably the fragment types).
--     Those live in NETWORK_TYPES_V1/_V2 below and are switched by the pref.
--   * The Mesh DHCP payloads (addr request/response) are parsed the same way in
--     both as far as is documented.
--   Entries marked `-- VERIFY` are my best reading of the TMRh20 source and
--   should be confirmed against the exact RF24Network/RF24Mesh release you target
--   before trusting them on the wire; the header decode does not depend on them.
-- ===========================================================================

local p_rf24 = Proto("rf24network", "RF24Network / RF24Mesh")

-- Message types stable across versions (the high-confidence set).
local NETWORK_TYPES_COMMON = {
  [128] = "NETWORK_ADDR_RESPONSE",
  [130] = "NETWORK_PING",
  [131] = "EXTERNAL_DATA_TYPE",
  [148] = "NETWORK_FIRST_FRAGMENT",
  [149] = "NETWORK_MORE_FRAGMENTS",
  [193] = "NETWORK_ACK",
  [194] = "NETWORK_POLL",
  [195] = "NETWORK_REQ_ADDRESS",
  [196] = "MESH_ADDR_LOOKUP",
  [197] = "MESH_ADDR_RELEASE",
  [198] = "MESH_ID_LOOKUP",
  [200] = "NETWORK_MORE_FRAGMENTS_NACK",
}
-- Per-version deltas, overlaid on COMMON. The last-fragment constant is the one
-- that moved between releases.
local NETWORK_TYPES_V2 = { [150] = "NETWORK_LAST_FRAGMENT" } -- VERIFY: 150 in current
local NETWORK_TYPES_V1 = { [201] = "NETWORK_LAST_FRAGMENT" } -- VERIFY: 201 in legacy

local function merged(base, delta)
  local t = {}
  for k, v in pairs(base) do t[k] = v end
  for k, v in pairs(delta) do t[k] = v end
  return t
end
local TYPES = { [1] = merged(NETWORK_TYPES_COMMON, NETWORK_TYPES_V1),
                [2] = merged(NETWORK_TYPES_COMMON, NETWORK_TYPES_V2) }

-- preference: which wire revision to assume (affects only the versioned types).
p_rf24.prefs.legacy =
  Pref.bool("Assume v1 (legacy) constants", false,
            "Use the RF24Network/RF24Mesh v1 message-type constants instead of v2 (current).")

local f = p_rf24.fields
f.from     = ProtoField.uint16("rf24network.from",     "From node", base.OCT)
f.to       = ProtoField.uint16("rf24network.to",       "To node",   base.OCT)
f.id       = ProtoField.uint16("rf24network.id",       "Sequence id", base.DEC)
f.type     = ProtoField.uint8 ("rf24network.type",     "Message type", base.DEC_HEX)
f.reserved = ProtoField.uint8 ("rf24network.reserved", "Reserved / fragment id", base.DEC)
f.address  = ProtoField.uint16("rf24network.mesh.address", "Assigned address", base.OCT)
f.node_id  = ProtoField.int16 ("rf24network.mesh.node_id", "Node ID", base.DEC)
f.payload  = ProtoField.bytes ("rf24network.payload", "Payload")

local function type_name(t, types)
  if types[t] then return types[t] end
  if t <= 127 then return string.format("User-defined (%d)", t) end
  return string.format("System (0x%02x)", t)
end

local function oct(v) return string.format("0%o", v) end

local function is_fragment(t)
  return t == 148 or t == 149 or t == 150 or t == 200 or t == 201
end

function p_rf24.dissector(tvb, pinfo, tree)
  local len = tvb:len()
  if len < 8 then return 0 end
  pinfo.cols.protocol = "RF24Mesh"

  local types = TYPES[p_rf24.prefs.legacy and 1 or 2]
  local from = tvb(0, 2):le_uint()
  local to = tvb(2, 2):le_uint()
  local id = tvb(4, 2):le_uint()
  local mtype = tvb(6, 1):uint()
  local tname = type_name(mtype, types)

  local st = tree:add(p_rf24, tvb(), string.format("RF24Network: %s → %s  %s", oct(from), oct(to), tname))
  st:add_le(f.from, tvb(0, 2)):append_text("  (" .. oct(from) .. ")")
  st:add_le(f.to, tvb(2, 2)):append_text("  (" .. oct(to) .. ")")
  st:add_le(f.id, tvb(4, 2))
  st:add(f.type, tvb(6, 1)):append_text("  (" .. tname .. ")")
  local rti = st:add(f.reserved, tvb(7, 1))
  if is_fragment(mtype) then
    rti:append_text("  (fragment #" .. tvb(7, 1):uint() .. ")")
  end

  -- payload (after the 8-byte header), interpreted by message type
  if len > 8 then
    local pl = tvb(8)
    if mtype == 128 and pl:len() >= 2 then
      -- NETWORK_ADDR_RESPONSE: master hands back the assigned logical address
      st:add_le(f.address, pl(0, 2)):append_text("  (" .. oct(pl(0, 2):le_uint()) .. ")")
      if pl:len() > 2 then st:add(f.payload, pl(2)) end
    elseif (mtype == 196 or mtype == 197 or mtype == 198) and pl:len() >= 2 then
      -- MESH_ADDR_LOOKUP / MESH_ADDR_RELEASE / MESH_ID_LOOKUP: a node ID
      st:add_le(f.node_id, pl(0, 2)) -- VERIFY payload shape per release
      if pl:len() > 2 then st:add(f.payload, pl(2)) end
    else
      st:add(f.payload, pl)
    end
  end

  pinfo.cols.src = oct(from)
  pinfo.cols.dst = oct(to)
  pinfo.cols.info = string.format("%s → %s  %s  id=%d", oct(from), oct(to), tname, id)
  return len
end

-- Bind to USER1 for standalone captures, and export by name for an ESB base
-- dissector to call (esb.dissector -> Dissector.get("rf24network"):call(...)).
local ok, wtab = pcall(function() return DissectorTable.get("wtap_encap") end)
if ok and wtab then
  pcall(function() wtab:add(wtap.USER1, p_rf24) end)
end
