Texture2D<float4> desktop : register(t0);
Texture2D<float4> pointer : register(t1);

struct Vertex {
    float4 position : SV_Position;
    float2 uv : TEXCOORD;
};

Vertex vertex(uint id : SV_VertexID) {
    Vertex v;
    v.uv = float2((id << 1) & 2, id & 2);
    v.position = float4(v.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return v;
}

float4 cursor_texel(Vertex v) {
    uint width, height;
    pointer.GetDimensions(width, height);
    return pointer.Load(int3(int2(v.uv * float2(width, height)), 0));
}

float4 color(Vertex v) : SV_Target {
    float4 sprite = cursor_texel(v);
    float3 base = desktop.Load(int3(int2(v.position.xy), 0)).rgb;
    return float4(lerp(base, sprite.rgb, sprite.a), 1);
}

float4 masked(Vertex v) : SV_Target {
    float4 sprite = cursor_texel(v);
    uint3 base = uint3(round(desktop.Load(int3(int2(v.position.xy), 0)).rgb * 255));
    uint3 ink = uint3(round(sprite.rgb * 255));
    return float4((sprite.a > 0.5 ? (base ^ ink) : ink) / 255.0, 1);
}

float4 monochrome(Vertex v) : SV_Target {
    float4 sprite = cursor_texel(v);
    uint3 base = uint3(round(desktop.Load(int3(int2(v.position.xy), 0)).rgb * 255));
    uint and_mask = sprite.r > 0.5 ? 255 : 0;
    uint xor_mask = sprite.g > 0.5 ? 255 : 0;
    return float4(((base & and_mask) ^ xor_mask) / 255.0, 1);
}
