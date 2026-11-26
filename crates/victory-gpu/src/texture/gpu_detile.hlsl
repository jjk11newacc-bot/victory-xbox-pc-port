ByteAddressBuffer tiled : register(t0);
RWByteAddressBuffer linear_rows : register(u0);
cbuffer Args : register(b0) {
    uint row_bytes, height, pitch, element_bytes;
    uint columns_at, rows_at, origin, render_xor;
    uint output_at;
};
uint address_at(uint x, uint b) {
    uint a = tiled.Load(columns_at + x * 4);
    return render_xor != 0 ? ((a & 0xffff0000) + (b & 0xffff0000) + ((a ^ b) & 0xffff)) : (a + b - origin);
}
[RootSignature("SRV(t0), UAV(u0), RootConstants(num32BitConstants=9,b0)")]
[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint byte_x = id.x * 4;
    if (byte_x >= row_bytes || id.y >= height) return;
    uint b = tiled.Load(rows_at + id.y * 4);
    uint value;
    if (element_bytes >= 4) {
        uint at = address_at(byte_x / element_bytes, b);
        value = tiled.Load(at + byte_x % element_bytes);
    } else {
        // One thread owns one aligned destination dword. Gather individual
        // texels from their aligned source dwords: no unaligned Load or
        // racing read-modify-write of neighboring output texels.
        value = 0;
        uint mask = element_bytes == 1 ? 255 : 65535;
        for (uint n = 0; n < 4; n += element_bytes) {
            if (byte_x + n >= row_bytes) break;
            uint at = address_at((byte_x + n) / element_bytes, b);
            uint texel = (tiled.Load(at & ~3u) >> ((at & 3u) * 8)) & mask;
            value |= texel << (n * 8);
        }
    }
    linear_rows.Store(output_at + id.y * pitch + byte_x, value);
}
