//! Direct3D 11 + DirectComposition GPU compositor for Lumen.
//!
//! Architecture:
//! 1. Shared wallpaper textures (`sharp_tex` + GPU-generated `blurred_tex` via
//!    Dual Kawase Blur) are computed once when the desktop wallpaper changes.
//! 2. A single pixel shader computes SDF rounded corners, fusion-free glass,
//!    liquid-glass rim refraction & chromatic aberration, nested glass plates
//!    (search bar, album art, play deck), grain overlay, and premultiplied
//!    content compositing in O(1) per-frame CPU overhead regardless of widget size.
//! 3. `GpuSurface` binds an `IDCompositionTarget` + `IDCompositionVisual` +
//!    bucketed `IDXGISwapChain2` (`SetSourceSize`) so pixel-by-pixel resizing
//!    does not reallocate GPU swapchains on every drag step.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{BOOL, HMODULE, HWND};

type HRESULT = i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

const IID_IDXGIDEVICE: GUID = GUID {
    data1: 0x54ec77fa,
    data2: 0x1377,
    data3: 0x44e6,
    data4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c],
};

const IID_IDXGIFACTORY2: GUID = GUID {
    data1: 0x50c83a1c,
    data2: 0xe072,
    data3: 0x4c48,
    data4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0],
};

const IID_ID3D11TEXTURE2D: GUID = GUID {
    data1: 0x6f15aaf2,
    data2: 0xd208,
    data3: 0x4e89,
    data4: [0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c],
};

const IID_IDCOMPOSITIONDEVICE: GUID = GUID {
    data1: 0xc37ea93a,
    data2: 0xe7aa,
    data3: 0x450d,
    data4: [0xb1, 0x6f, 0x97, 0x46, 0xcb, 0x04, 0x07, 0xf3],
};

const IID_IDXGISWAPCHAIN2: GUID = GUID {
    data1: 0xa8be2ac4,
    data2: 0x199f,
    data3: 0x4946,
    data4: [0xb3, 0x31, 0x79, 0x59, 0x9f, 0xb9, 0x8d, 0xe7],
};

const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
const D3D11_SDK_VERSION: u32 = 7;
const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
const D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;

const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
const DXGI_SCALING_STRETCH: u32 = 0;
#[allow(dead_code)]
const DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL: u32 = 3;
const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;
const DXGI_ALPHA_MODE_PREMULTIPLIED: u32 = 1;

const D3D11_USAGE_DEFAULT: u32 = 0;
const D3D11_USAGE_DYNAMIC: u32 = 2;
const D3D11_USAGE_STAGING: u32 = 3;
const D3D11_BIND_SHADER_RESOURCE: u32 = 8;
const D3D11_BIND_RENDER_TARGET: u32 = 32;
const D3D11_BIND_CONSTANT_BUFFER: u32 = 4;
const D3D11_CPU_ACCESS_WRITE: u32 = 0x10000;
const D3D11_CPU_ACCESS_READ: u32 = 0x20000;
const D3D11_MAP_READ: u32 = 1;
const D3D11_MAP_WRITE_DISCARD: u32 = 4;

const D3D11_FILTER_MIN_MAG_MIP_LINEAR: u32 = 0x15;
const D3D11_TEXTURE_ADDRESS_CLAMP: u32 = 3;
const D3D11_TEXTURE_ADDRESS_WRAP: u32 = 1;

#[repr(C)]
struct D3D11BufferDesc {
    byte_width: u32,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
    structure_byte_stride: u32,
}

#[repr(C)]
struct D3D11Texture2dDesc {
    width: u32,
    height: u32,
    mip_levels: u32,
    array_size: u32,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
}

#[repr(C)]
struct D3D11SubresourceData {
    p_sys_mem: *const c_void,
    sys_mem_pitch: u32,
    sys_mem_slice_pitch: u32,
}

#[repr(C)]
struct D3D11MappedSubresource {
    p_data: *mut c_void,
    row_pitch: u32,
    depth_pitch: u32,
}

#[repr(C)]
struct D3D11Viewport {
    top_left_x: f32,
    top_left_y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}

#[repr(C)]
struct D3D11SamplerDesc {
    filter: u32,
    address_u: u32,
    address_v: u32,
    address_w: u32,
    mip_lod_bias: f32,
    max_anisotropy: u32,
    comparison_func: u32,
    border_color: [f32; 4],
    min_lod: f32,
    max_lod: f32,
}

#[repr(C)]
struct DxgiSwapChainDesc1 {
    width: u32,
    height: u32,
    format: u32,
    stereo: BOOL,
    sample_count: u32,
    sample_quality: u32,
    buffer_usage: u32,
    buffer_count: u32,
    scaling: u32,
    swap_effect: u32,
    alpha_mode: u32,
    flags: u32,
}

#[link(name = "d3d11")]
extern "system" {
    fn D3D11CreateDevice(
        p_adapter: *mut c_void,
        driver_type: u32,
        software: HMODULE,
        flags: u32,
        p_feature_levels: *const u32,
        feature_levels: u32,
        sdk_version: u32,
        pp_device: *mut *mut c_void,
        p_feature_level: *mut u32,
        pp_immediate_context: *mut *mut c_void,
    ) -> HRESULT;
}

#[link(name = "dcomp")]
extern "system" {
    fn DCompositionCreateDevice(
        dxgi_device: *mut c_void,
        iid: *const GUID,
        dcomposition_device: *mut *mut c_void,
    ) -> HRESULT;
}

#[link(name = "d3dcompiler")]
extern "system" {
    fn D3DCompile(
        p_src_data: *const u8,
        src_data_size: usize,
        p_source_name: *const u8,
        p_defines: *const c_void,
        p_include: *const c_void,
        p_entry_point: *const u8,
        p_target: *const u8,
        flags1: u32,
        flags2: u32,
        pp_code: *mut *mut c_void,
        pp_error_msgs: *mut *mut c_void,
    ) -> HRESULT;
}

/// Safe RAII wrapper for a raw COM interface pointer.
pub struct ComPtr(*mut c_void);

impl ComPtr {
    #[inline]
    fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    #[inline]
    unsafe fn vtable(&self) -> *const *const c_void {
        *(self.0 as *const *const *const c_void)
    }

    unsafe fn query_interface(&self, iid: &GUID) -> Option<ComPtr> {
        let f: extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT =
            std::mem::transmute(*self.vtable().add(0));
        let mut out = null_mut();
        if f(self.0, iid, &mut out) >= 0 && !out.is_null() {
            Some(ComPtr(out))
        } else {
            None
        }
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let release: extern "system" fn(*mut c_void) -> u32 =
                    std::mem::transmute(*self.vtable().add(2));
                release(self.0);
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NestedGlassRect {
    pub rect: [f32; 4], // l, t, r, b in widget pixel coordinates
    pub radius: f32,
    pub tint_alpha: f32,
    pub _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlassConstants {
    pub win_rect: [f32; 4],   // x, y, w, h in desktop pixels
    pub screen_size: [f32; 2], // desktop w, h in pixels
    pub radius: f32,
    pub dpi: f32,
    pub tint_color: [f32; 4], // linear/sRGB r, g, b, tint_alpha
    pub flags: [f32; 4],      // [frosted, liquid, frost_strength, opacity]
    pub nested_count: u32,
    pub has_content: u32,
    pub fuse_line_count: u32,
    pub fuse_corner_count: u32,
    pub content_uv_scale: [f32; 4],
    pub nested: [NestedGlassRect; 8],
    pub fuse_lines: [[f32; 4]; 32],
    pub fuse_corner_c: [[f32; 4]; 32],
    pub fuse_corner_uv: [[f32; 4]; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FusionBridgeConstants {
    pub bridge_rect: [f32; 4],          // l, t, w, h in desktop pixels
    pub desktop_rect: [f32; 4],         // origin_x, origin_y, desktop_w, desktop_h
    pub params: [f32; 4],               // [dpi, pad_f, liquid, morph_k]
    pub accent_flags: [f32; 4],         // [accent_r, accent_g, accent_b, has_wallpaper]
    pub counts: [u32; 4],               // [pane_count, member_win_count, fuse_line_count, fuse_corner_count]
    pub pane_rects: [[f32; 4]; 16],     // l, t, r, b in local bridge pixels
    pub pane_meta: [[f32; 4]; 16],      // [radius, opacity, dpi, 0.0]
    pub pane_delta: [[f32; 4]; 16],     // [dr, dg, db, has_delta] in -1.0..1.0
    pub member_windows: [[f32; 4]; 16], // l, t, r, b in local bridge pixels
    pub fuse_lines: [[f32; 4]; 32],     // x0, y0, x1, y1 in local bridge pixels
    pub fuse_corner_c: [[f32; 4]; 32],  // cx, cy, radius, +1 convex / -1 concave
    pub fuse_corner_uv: [[f32; 4]; 32], // -v.x, -v.y, u.x, u.y
    pub field_uv: [f32; 4],             // [tex_w, tex_h, step_px, active]
    pub blur_params: [f32; 4],          // [blur_px, 0.0, 0.0, 0.0]
}

impl Default for FusionBridgeConstants {
    fn default() -> Self {
        Self {
            bridge_rect: [0.0; 4],
            desktop_rect: [0.0; 4],
            params: [1.0, 48.0, 0.0, 0.0],
            accent_flags: [0.0; 4],
            counts: [0; 4],
            pane_rects: [[0.0; 4]; 16],
            pane_meta: [[0.0; 4]; 16],
            pane_delta: [[0.0; 4]; 16],
            member_windows: [[0.0; 4]; 16],
            fuse_lines: [[0.0; 4]; 32],
            fuse_corner_c: [[0.0; 4]; 32],
            fuse_corner_uv: [[0.0; 4]; 32],
            field_uv: [0.0; 4],
            blur_params: [0.0; 4],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlurConstants {
    texel_size: [f32; 2],
    offset: f32,
    _pad: f32,
}

const SHADER_SRC: &str = r#"
cbuffer GlassCB : register(b0) {
    float4 win_rect;
    float2 screen_size;
    float radius;
    float dpi;
    float4 tint_color;
    float4 flags; // x=frosted, y=liquid, z=frost_strength, w=opacity
    uint nested_count;
    uint has_content;
    uint fuse_line_count;
    uint fuse_corner_count;
    float4 content_uv_scale;
    float4 nested_rect[8];
    float4 nested_meta[8]; // x=radius, y=tint_alpha
    float4 fuse_lines[32];     // x0, y0, x1, y1 in local window px
    float4 fuse_corner_c[32];  // cx, cy, radius, +1 convex / -1 concave
    float4 fuse_corner_uv[32]; // -v.x, -v.y, u.x, u.y
};

cbuffer BlurCB : register(b1) {
    float2 texel_size;
    float blur_offset;
    float _bpad;
};

cbuffer FusionBridgeCB : register(b2) {
    float4 fb_bridge_rect;
    float4 fb_desktop_rect;
    float4 fb_params;
    float4 fb_accent_flags;
    uint4 fb_counts;
    float4 fb_pane_rects[16];
    float4 fb_pane_meta[16];
    float4 fb_pane_delta[16];
    float4 fb_member_windows[16];
    float4 fb_fuse_lines[32];
    float4 fb_fuse_corner_c[32];
    float4 fb_fuse_corner_uv[32];
    float4 fb_field_uv;
    float4 fb_blur_params;
};

Texture2D sharp_tex : register(t0);
Texture2D blurred_tex : register(t1);
#ifdef FUSION_BRIDGE_PS
Texture2D bridge_field_tex : register(t2);
#else
Texture2D content_tex : register(t2);
#define bridge_field_tex content_tex
#endif
Texture2D grain_tex : register(t3);
SamplerState samp_clamp : register(s0);
SamplerState samp_wrap : register(s1);

struct VSOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VSOut fullscreen_vs(uint id : SV_VertexID) {
    VSOut o;
    float2 uv = float2((id << 1) & 2, id & 2);
    o.uv = uv;
    o.pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}

float4 kawase_down_ps(VSOut i) : SV_Target {
    float2 o = texel_size * blur_offset;
    float4 s = sharp_tex.Sample(samp_clamp, i.uv) * 4.0;
    s += sharp_tex.Sample(samp_clamp, i.uv - o);
    s += sharp_tex.Sample(samp_clamp, i.uv + o);
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(o.x, -o.y));
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(-o.x, o.y));
    return s / 8.0;
}

float4 kawase_up_ps(VSOut i) : SV_Target {
    float2 o = texel_size * blur_offset;
    float4 s = sharp_tex.Sample(samp_clamp, i.uv + float2(-o.x * 2.0, 0.0));
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(-o.x, o.y)) * 2.0;
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(0.0, o.y * 2.0));
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(o.x, o.y)) * 2.0;
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(o.x * 2.0, 0.0));
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(o.x, -o.y)) * 2.0;
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(0.0, -o.y * 2.0));
    s += sharp_tex.Sample(samp_clamp, i.uv + float2(-o.x, -o.y)) * 2.0;
    return s / 12.0;
}

float sd_round_rect(float2 p, float2 half_size, float r) {
    float2 q = abs(p) - (half_size - r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

// The fusion variant binds t2 as bridge_field_tex. HLSL validates even
// non-entry functions, so it must not see the widget-only content_tex access.
#ifndef FUSION_BRIDGE_PS
float4 glass_composite_ps(VSOut i) : SV_Target {
    float2 local_px = i.uv * win_rect.zw;
    float2 half_size = win_rect.zw * 0.5;
    float d = sd_round_rect(local_px - half_size, half_size, radius);
    float outer_cov = saturate(0.5 - d);
    if (outer_cov <= 0.0) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }

    float2 screen_uv = (win_rect.xy + local_px) / max(screen_size, float2(1.0, 1.0));
    float3 base_rgb = tint_color.rgb;

    if (flags.x > 0.5) {
        float3 sharp_col = sharp_tex.Sample(samp_clamp, screen_uv).rgb;
        float3 blur_col = blurred_tex.Sample(samp_clamp, screen_uv).rgb;
        float3 wall = lerp(sharp_col, blur_col, saturate(flags.z));

        // Liquid glass rim refraction + subtle chromatic aberration
        if (flags.y > 0.5) {
            float rim_w = clamp(22.0 * dpi, 8.0, min(half_size.x, half_size.y) * 0.45);
            if (d > -rim_w) {
                float t = saturate(1.0 + d / rim_w);
                float2 grad = normalize(local_px - half_size + 1e-4);
                float2 refract_uv = screen_uv - grad * (t * t * 10.0 * dpi / max(screen_size, float2(1.0, 1.0)));
                float2 ca = grad * (t * 7.5 * dpi / max(screen_size, float2(1.0, 1.0)));
                wall.r = blurred_tex.Sample(samp_clamp, refract_uv + ca).r;
                wall.g = blurred_tex.Sample(samp_clamp, refract_uv).g;
                wall.b = blurred_tex.Sample(samp_clamp, refract_uv - ca).b;
                float rim_spec = pow(t, 2.2) * 0.22;
                wall = saturate(wall + rim_spec);
            }
        }
        base_rgb = lerp(wall, tint_color.rgb, saturate(tint_color.a));
    }

    // Nested glass plates (search bar, album art, play deck)
    [loop]
    for (uint n = 0; n < min(nested_count, 8u); n++) {
        float4 nr = nested_rect[n];
        float2 nh = (nr.zw - nr.xy) * 0.5;
        float2 nc = (nr.xy + nr.zw) * 0.5;
        float nd = sd_round_rect(local_px - nc, nh, nested_meta[n].x);
        float ncov = saturate(0.5 - nd);
        if (ncov > 0.0) {
            float3 nblur = blurred_tex.Sample(samp_clamp, screen_uv).rgb;
            float3 ncol = lerp(nblur, tint_color.rgb, saturate(nested_meta[n].y));
            float nspec = saturate(1.0 - abs(nd) / max(1.5 * dpi, 1.0)) * 0.14;
            ncol = saturate(ncol + nspec);
            base_rgb = lerp(base_rgb, ncol, ncov);
        }
    }

    // Subtle grain tiling
    float grain = grain_tex.Sample(samp_wrap, local_px / 128.0).r;
    base_rgb = saturate(base_rgb + (grain - 0.5) * 0.025);

    float4 out_col = float4(base_rgb * outer_cov, outer_cov) * flags.w;

    // Premultiplied content layer composite
    if (has_content > 0u) {
        float4 c = content_tex.Sample(samp_clamp, min(i.uv * content_uv_scale.xy, content_uv_scale.zw));
        if (flags.w <= 0.001) {
            return c;
        }
        out_col = c + out_col * (1.0 - c.a);
    }

    return out_col;
}

#endif

float fb_smin(float a, float b, float k) {
    float h = saturate(0.5 + 0.5 * (b - a) / max(k, 1e-4));
    return lerp(b, a, h) - k * h * (1.0 - h);
}

float fb_contour_sdf(float2 p) {
    uint lc = min(fb_counts.z, 32u);
    uint cc = min(fb_counts.w, 32u);
    float d_sq = 1e20;
    bool inside = false;

    [loop]
    for (uint li = 0u; li < lc; li++) {
        float4 seg = fb_fuse_lines[li];
        float2 a = seg.xy;
        float2 b = seg.zw;
        float2 ab = b - a;
        float len2 = dot(ab, ab);
        float t = (len2 > 1e-8) ? saturate(dot(p - a, ab) / len2) : 0.0;
        float2 diff = p - (a + ab * t);
        d_sq = min(d_sq, dot(diff, diff));

        if (abs(ab.y) > abs(ab.x)) {
            float y0 = min(a.y, b.y);
            float y1 = max(a.y, b.y);
            if (p.y >= y0 && p.y < y1 && a.x > p.x) {
                inside = !inside;
            }
        }
    }

    float d = sqrt(d_sq);
    bool corner_override = false;
    bool corner_inside = false;

    [loop]
    for (uint ci = 0u; ci < cc; ci++) {
        float4 cc4 = fb_fuse_corner_c[ci];
        float4 cuv = fb_fuse_corner_uv[ci];
        float2 center = cc4.xy;
        float r = cc4.z;
        float convex = cc4.w;
        float2 neg_v = cuv.xy;
        float2 u_vec = cuv.zw;

        float2 v_pt = center + (u_vec + neg_v) * r;
        float2 cmin = min(v_pt, center);
        float2 cmax = max(v_pt, center);
        if (p.y >= cmin.y && p.y < cmax.y) {
            float sx = u_vec.x + neg_v.x;
            float dy = p.y - center.y;
            float dx = sqrt(max(0.0, r * r - dy * dy));
            if (center.x + sx * dx > p.x) {
                inside = !inside;
            }
        }

        float2 q = p - center;
        float box_d = max(abs(q.x) - r, abs(q.y) - r);
        if (!(box_d > 0.0 && box_d >= d)) {
            float q_len = length(q);
            bool on_quad = (dot(q, neg_v) >= 0.0) && (dot(q, u_vec) >= 0.0);
            if (on_quad) {
                d = min(d, abs(q_len - r));
            } else {
                float2 pa = p - (center + neg_v * r);
                float2 pb = p - (center + u_vec * r);
                d = min(d, sqrt(min(dot(pa, pa), dot(pb, pb))));
            }
            if (p.x >= cmin.x && p.x <= cmax.x && p.y >= cmin.y && p.y <= cmax.y) {
                corner_override = true;
                corner_inside = (convex > 0.0) ? (q_len <= r) : (q_len >= r);
            }
        }
    }

    if (corner_override) {
        inside = corner_inside;
    }
    return inside ? -d : d;
}

float fb_smin_sdf(float2 p, float k) {
    uint pc = min(fb_counts.x, 16u);
    if (pc == 0u) {
        return 1e6;
    }
    float d = 1e6;
    [loop]
    for (uint j = 0u; j < pc; j++) {
        float4 pr = fb_pane_rects[j];
        float2 ph = (pr.zw - pr.xy) * 0.5;
        float2 pcen = (pr.xy + pr.zw) * 0.5;
        float dj = sd_round_rect(p - pcen, ph, fb_pane_meta[j].x);
        d = (j == 0u) ? dj : fb_smin(d, dj, k);
    }
    return d;
}

float fb_sdf(float2 p) {
    float morph_k = fb_params.w;
    bool has_contour = (fb_counts.z > 0u || fb_counts.w > 0u);
    if (morph_k > 0.001 && fb_counts.x > 0u) {
        return fb_smin_sdf(p, morph_k);
    }
    if (has_contour) {
        return fb_contour_sdf(p);
    }
    return fb_smin_sdf(p, 0.0);
}

float fb_sample_ca_channel(
    Texture2D tex,
    float2 screen_uv,
    float2 n,
    float2 desk_size,
    float disp,
    float signed_ca,
    float fb_dpi,
    float3 ch_mask
) {
    if (abs(signed_ca) <= 0.05) {
        return dot(tex.Sample(samp_clamp, screen_uv + n * disp / desk_size).rgb, ch_mask);
    }
    float abs_ca = abs(signed_ca);
    float trans = 0.75 * max(fb_dpi, 1.0);
    float t_fp = saturate((abs_ca - 0.05) / trans);
    float fp = t_fp * t_fp * t_fp * (t_fp * (t_fp * 6.0 - 15.0) + 10.0);
    float soft_ca = (signed_ca >= 0.0 ? 1.0 : -1.0) * (abs_ca - 0.05);
    float d0 = disp + soft_ca * (1.0 - 0.35 * fp);
    float d1 = disp + soft_ca;
    float d2 = disp + soft_ca * (1.0 + 0.35 * fp);
    float spread = max(abs_ca * 0.45, 0.75 * fb_dpi) * fp;
    float2 t_uv = float2(-n.y, n.x) * spread / desk_size;
    float2 n_uv = n / desk_size;
    return 0.20 * dot(tex.Sample(samp_clamp, screen_uv + n_uv * d0).rgb, ch_mask)
         + 0.30 * dot(tex.Sample(samp_clamp, screen_uv + n_uv * d1).rgb, ch_mask)
         + 0.15 * dot(tex.Sample(samp_clamp, screen_uv + n_uv * d1 + t_uv).rgb, ch_mask)
         + 0.15 * dot(tex.Sample(samp_clamp, screen_uv + n_uv * d1 - t_uv).rgb, ch_mask)
         + 0.20 * dot(tex.Sample(samp_clamp, screen_uv + n_uv * d2).rgb, ch_mask);
}

float4 fusion_bridge_ps(VSOut i) : SV_Target {
    float2 local_px = i.uv * fb_bridge_rect.zw;
    float fb_dpi = max(fb_params.x, 1.0);
    float pad_f = max(fb_params.y, 1.0);

    float d = fb_sdf(local_px);
    if (d >= pad_f) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }

    float coverage = saturate(0.5 - d);
    float underlap = 3.0 * fb_dpi;
    uint wc = min(fb_counts.y, 16u);
    bool in_window = false;
    bool in_deep_window = false;
    bool in_deep_opaque_window = false;
    bool uniform_op = true;
    float op0 = (fb_counts.x > 0u) ? fb_pane_meta[0].y : 1.0;
    [loop]
    for (uint wi = 0u; wi < wc; wi++) {
        float w_op = fb_pane_meta[wi].y;
        if (abs(w_op - op0) > 1e-4) {
            uniform_op = false;
        }
        float4 mw = fb_member_windows[wi];
        if (local_px.x >= mw.x && local_px.x < mw.z && local_px.y >= mw.y && local_px.y < mw.w) {
            in_window = true;
            if (local_px.x >= mw.x + underlap && local_px.x < mw.z - underlap
                && local_px.y >= mw.y + underlap && local_px.y < mw.w - underlap)
            {
                in_deep_window = true;
                if (w_op >= 0.999) {
                    in_deep_opaque_window = true;
                }
            }
        }
    }
    if ((in_window && d >= -1.0 && coverage > 0.0)
        || (in_deep_opaque_window && d < -1.0)
        || (in_deep_window && d < -1.0 && uniform_op))
    {
        return float4(0.0, 0.0, 0.0, 0.0);
    }

    float alpha = 1.0;
    float fg_alpha = 1.0;
    float3 delta_rgb = float3(0.0, 0.0, 0.0);
    float frost_mix = 1.0;
    uint pc = min(fb_counts.x, 16u);
    if (pc > 0u) {
        float sum_w = 0.0;
        float sum_a = 0.0;
        float sum_fg_w = 0.0;
        float sum_fg_a = 0.0;
        float reach = 120.0 * fb_dpi;
        float3 sum_delta = float3(0.0, 0.0, 0.0);
        float sum_wf = 0.0;
        float sum_sharp = 0.0;
        [loop]
        for (uint pi = 0u; pi < pc; pi++) {
            float4 pr = fb_pane_rects[pi];
            float2 ph = (pr.zw - pr.xy) * 0.5;
            float2 pcen = (pr.xy + pr.zw) * 0.5;
            float dp = sd_round_rect(local_px - pcen, ph, fb_pane_meta[pi].x);
            float w = exp(-max(dp - d, 0.0) / (20.0 * fb_dpi));
            sum_w += w;
            sum_a += w * fb_pane_meta[pi].y;
            sum_delta += w * fb_pane_delta[pi].rgb * fb_pane_delta[pi].w;

            float pdpi = max(fb_pane_meta[pi].z, 1.0);
            float srad = min(4.0 * pdpi, min(ph.x, ph.y));
            float d_seam = max(sd_round_rect(local_px - pcen, ph, srad), 0.0);
            if (d_seam < reach) {
                float t_fg = 1.0 - d_seam / reach;
                float w_fg = t_fg * t_fg;
                sum_fg_w += w_fg;
                sum_fg_a += w_fg * fb_pane_meta[pi].y;
            }
            float wf = exp(-d_seam / (96.0 * pdpi));
            sum_wf += wf;
            sum_sharp += wf * fb_pane_meta[pi].w;
        }
        alpha = (sum_w > 0.0) ? (sum_a / sum_w) : 0.0;
        if (uniform_op) {
            fg_alpha = alpha;
        } else if (in_window) {
            fg_alpha = 0.0;
        } else {
            fg_alpha = (sum_fg_w > 0.0) ? (sum_fg_a / sum_fg_w) : 0.0;
        }
        delta_rgb = (sum_w > 0.0) ? (sum_delta / sum_w) : float3(0.0, 0.0, 0.0);
        frost_mix = 1.0 - ((sum_wf > 0.0) ? saturate(sum_sharp / sum_wf) : 0.0);
    }

    float sd = fb_sdf(local_px - float2(5.0 * fb_dpi, 8.0 * fb_dpi));
    float fade = saturate((pad_f - d) / (8.0 * fb_dpi));
    float tx = clamp(sd / (17.0 * fb_dpi), -4.8, 4.8);
    float tx2 = tx * tx;
    float th = clamp(
        tx * (135135.0 + tx2 * (17325.0 + tx2 * (378.0 + tx2)))
            / (135135.0 + tx2 * (62370.0 + tx2 * (3150.0 + tx2 * 28.0))),
        -1.0,
        1.0
    );
    float shadow_a = saturate(0.5 * (1.0 - th) * (96.0 / 255.0) * alpha * fade);
    if (coverage <= 0.0) {
        return float4(0.0, 0.0, 0.0, shadow_a);
    }

    float dx = fb_sdf(local_px + float2(0.5, 0.0)) - fb_sdf(local_px - float2(0.5, 0.0));
    float dy = fb_sdf(local_px + float2(0.0, 0.5)) - fb_sdf(local_px - float2(0.0, 0.5));
    float2 grad = float2(dx, dy);
    float glen = max(length(grad), 1e-4);
    float2 n = grad / glen;

    float band = 2.5 * fb_dpi;
    float rim_edge = (d >= -band)
        ? ((0.09 + 0.28 * max(-n.x * 0.6 - n.y * 0.8, 0.0)) * saturate(1.0 + min(d, 0.0) / band))
        : 0.0;

    float liquid_spec = 0.0;
    if (fb_params.z > 0.5 && d <= 0.5) {
        float spec_w = 0.32 * max(min(44.0 * fb_dpi, min(fb_bridge_rect.z, fb_bridge_rect.w) * 0.45), 1.0);
        if (-d < spec_w) {
            float ledge = saturate(1.0 - max(-d, 0.0) / spec_w);
            float light_dir = max(-n.x * 0.55 - n.y * 0.83, 0.0) + max(n.x * 0.4 + n.y * 0.6, 0.0) * 0.35;
            liquid_spec = (ledge * ledge * ledge * (16.0 + 34.0 * light_dir)) / 255.0;
        }
    }

    float3 wall = fb_accent_flags.rgb;
    if (fb_accent_flags.w > 0.5) {
        float2 desk_size = max(fb_desktop_rect.zw, float2(1.0, 1.0));
        float2 screen_uv = (fb_bridge_rect.xy + local_px - fb_desktop_rect.xy) / desk_size;
        float3 blur_base = blurred_tex.Sample(samp_clamp, screen_uv).rgb;
        wall = (frost_mix < 0.999)
            ? lerp(sharp_tex.Sample(samp_clamp, screen_uv).rgb, blur_base, frost_mix)
            : blur_base;

        if (fb_params.z > 0.5) {
            float bridge_rim = clamp(40.0 * fb_dpi, 12.0, min(fb_bridge_rect.z, fb_bridge_rect.w) * 0.40);
            if (d > -bridge_rim) {
                float u = saturate(-d / bridge_rim);
                float k = saturate(1.0 - u);
                float edge_align = k * k * (2.0 - k);
                float ca_factor = 0.88 * edge_align;
                float lens_strength = min(1.0, 0.65 * bridge_rim / ((4.0 / 3.0) * (12.5 + 1.65 * 4.5) * fb_dpi));
                float disp = -12.5 * fb_dpi * edge_align * lens_strength;
                float ca = ca_factor * 4.5 * fb_dpi * lens_strength;

                float2 refract_uv = screen_uv + n * disp / desk_size;
                float3 blur_col;
                blur_col.r = fb_sample_ca_channel(blurred_tex, screen_uv, n, desk_size, disp, -ca, fb_dpi, float3(1.0, 0.0, 0.0));
                blur_col.g = blurred_tex.Sample(samp_clamp, refract_uv).g;
                blur_col.b = fb_sample_ca_channel(blurred_tex, screen_uv, n, desk_size, disp, ca, fb_dpi, float3(0.0, 0.0, 1.0));
                if (frost_mix < 0.999) {
                    float3 sharp_refr;
                    sharp_refr.r = fb_sample_ca_channel(sharp_tex, screen_uv, n, desk_size, disp, -ca, fb_dpi, float3(1.0, 0.0, 0.0));
                    sharp_refr.g = sharp_tex.Sample(samp_clamp, refract_uv).g;
                    sharp_refr.b = fb_sample_ca_channel(sharp_tex, screen_uv, n, desk_size, disp, ca, fb_dpi, float3(0.0, 0.0, 1.0));
                    wall = lerp(sharp_refr, blur_col, frost_mix);
                } else {
                    wall = blur_col;
                }

                float bt_raw = saturate((fb_blur_params.x - 10.0) / 70.0);
                float blur_t = bt_raw * (2.0 - bt_raw);
                float clarity_reach = 0.88 - 0.48 * blur_t;
                if (u < clarity_reach) {
                    float s = saturate(1.0 - u / clarity_reach);
                    float f = s * s * s * (s * (s * 6.0 - 15.0) + 10.0);
                    float f_eff = f * (1.0 + blur_t * (f * f - 1.0));
                    float clarity = 0.52 * (1.0 - 0.72 * blur_t) * f_eff;
                    float ca_s = ca * 0.55;
                    float3 sharp_col;
                    sharp_col.r = fb_sample_ca_channel(sharp_tex, screen_uv, n, desk_size, disp, -ca_s, fb_dpi, float3(1.0, 0.0, 0.0));
                    sharp_col.g = sharp_tex.Sample(samp_clamp, refract_uv).g;
                    sharp_col.b = fb_sample_ca_channel(sharp_tex, screen_uv, n, desk_size, disp, ca_s, fb_dpi, float3(0.0, 0.0, 1.0));
                    wall = lerp(wall, sharp_col, clarity);
                }
            }
        }
    }
    if (fb_field_uv.x > 0.0 && fb_field_uv.y > 0.0 && fb_field_uv.z > 0.0) {
        float2 field_uv = (local_px / fb_field_uv.z + 0.5) / fb_field_uv.xy;
        float4 gf = bridge_field_tex.Sample(samp_clamp, field_uv);
        if (fb_field_uv.w > 1.5) {
            wall = saturate(wall * gf.a + gf.rgb);
        } else {
            wall = saturate(wall * gf.a + (gf.rgb - 0.5) * 2.0);
        }
    } else {
        wall = saturate(wall + delta_rgb);
    }

    float3 lit = min(wall + (1.0 - wall) * rim_edge + liquid_spec, 1.0);
    float fg_a = coverage * fg_alpha;
    float out_a = fg_a + shadow_a * (1.0 - fg_a);
    return float4(lit * fg_a, out_a);
}
"#;

pub struct GpuDevice {
    pub device: ComPtr,
    pub context: ComPtr,
    pub dxgi_factory: ComPtr,
    pub dcomp: ComPtr,
    vs: ComPtr,
    ps_down: ComPtr,
    ps_up: ComPtr,
    ps_glass: ComPtr,
    ps_fusion: ComPtr,
    samp_clamp: ComPtr,
    samp_wrap: ComPtr,
    glass_cb: ComPtr,
    blur_cb: ComPtr,
    fusion_cb: ComPtr,
    grain_srv: ComPtr,
    sharp_tex: Option<(i32, i32, ComPtr, ComPtr)>,   // w, h, tex, srv
    blurred_tex: Option<(i32, i32, ComPtr, ComPtr)>, // w, h, tex, srv
}

impl GpuDevice {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut dev_ptr = null_mut();
            let mut ctx_ptr = null_mut();
            let levels = [D3D_FEATURE_LEVEL_11_0];
            let hr = D3D11CreateDevice(
                null_mut(),
                D3D_DRIVER_TYPE_HARDWARE,
                null_mut(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                levels.as_ptr(),
                levels.len() as u32,
                D3D11_SDK_VERSION,
                &mut dev_ptr,
                null_mut(),
                &mut ctx_ptr,
            );
            if hr < 0 || dev_ptr.is_null() || ctx_ptr.is_null() {
                return None;
            }
            let device = ComPtr(dev_ptr);
            let context = ComPtr(ctx_ptr);
            let dxgi_device = device.query_interface(&IID_IDXGIDEVICE)?;

            // Get IDXGIAdapter -> IDXGIFactory2
            let get_adapter: extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT =
                std::mem::transmute(*dxgi_device.vtable().add(7));
            let mut adapter_ptr = null_mut();
            if get_adapter(dxgi_device.as_ptr(), &mut adapter_ptr) < 0 || adapter_ptr.is_null() {
                return None;
            }
            let adapter = ComPtr(adapter_ptr);
            let get_parent: extern "system" fn(
                *mut c_void,
                *const GUID,
                *mut *mut c_void,
            ) -> HRESULT = std::mem::transmute(*adapter.vtable().add(6));
            let mut factory_ptr = null_mut();
            if get_parent(adapter.as_ptr(), &IID_IDXGIFACTORY2, &mut factory_ptr) < 0
                || factory_ptr.is_null()
            {
                return None;
            }
            let dxgi_factory = ComPtr(factory_ptr);

            let mut dcomp_ptr = null_mut();
            if DCompositionCreateDevice(
                dxgi_device.as_ptr(),
                &IID_IDCOMPOSITIONDEVICE,
                &mut dcomp_ptr,
            ) < 0
                || dcomp_ptr.is_null()
            {
                return None;
            }
            let dcomp = ComPtr(dcomp_ptr);

            let vs_blob = compile_shader("fullscreen_vs\0", "vs_5_0\0")?;
            let ps_down_blob = compile_shader("kawase_down_ps\0", "ps_5_0\0")?;
            let ps_up_blob = compile_shader("kawase_up_ps\0", "ps_5_0\0")?;
            let ps_glass_blob = compile_shader("glass_composite_ps\0", "ps_5_0\0")?;
            let ps_fusion_blob = compile_shader("fusion_bridge_ps\0", "ps_5_0\0")?;

            let vs = create_vs(&device, &vs_blob)?;
            let ps_down = create_ps(&device, &ps_down_blob)?;
            let ps_up = create_ps(&device, &ps_up_blob)?;
            let ps_glass = create_ps(&device, &ps_glass_blob)?;
            let ps_fusion = create_ps(&device, &ps_fusion_blob)?;

            let samp_clamp = create_sampler(&device, D3D11_TEXTURE_ADDRESS_CLAMP)?;
            let samp_wrap = create_sampler(&device, D3D11_TEXTURE_ADDRESS_WRAP)?;
            let glass_cb = create_cb(&device, size_of::<GlassConstants>() as u32)?;
            let blur_cb = create_cb(&device, size_of::<BlurConstants>() as u32)?;
            let fusion_cb = create_cb(&device, size_of::<FusionBridgeConstants>() as u32)?;
            let grain_srv = create_grain_srv(&device)?;

            Some(Self {
                device,
                context,
                dxgi_factory,
                dcomp,
                vs,
                ps_down,
                ps_up,
                ps_glass,
                ps_fusion,
                samp_clamp,
                samp_wrap,
                glass_cb,
                blur_cb,
                fusion_cb,
                grain_srv,
                sharp_tex: None,
                blurred_tex: None,
            })
        }
    }

    /// Upload a desktop wallpaper (BGRA) once and generate its half-res Dual
    /// Kawase blurred texture on the GPU.
    pub unsafe fn set_wallpaper_bgra(&mut self, w: i32, h: i32, bgra: &[u8]) -> bool {
        if w <= 0 || h <= 0 || bgra.len() < (w * h * 4) as usize {
            return false;
        }
        let Some((sharp_tex, sharp_srv)) =
            create_texture2d(&self.device, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, Some(bgra), false)
        else {
            return false;
        };

        let bw = (w / 2).max(1);
        let bh = (h / 2).max(1);
        let Some((down_tex, down_srv, down_rtv)) = create_rt_texture(&self.device, bw, bh) else {
            return false;
        };
        let Some((blur_tex, blur_srv, blur_rtv)) = create_rt_texture(&self.device, bw, bh) else {
            return false;
        };

        // Pass 1: Kawase downsample sharp -> down_tex
        let cb1 = BlurConstants {
            texel_size: [1.0 / w as f32, 1.0 / h as f32],
            offset: 1.5,
            _pad: 0.0,
        };
        self.run_blur_pass(&sharp_srv, &down_rtv, bw, bh, &self.ps_down, &cb1);

        // Pass 2: Kawase upsample/refine down_tex -> blur_tex
        let cb2 = BlurConstants {
            texel_size: [1.0 / bw as f32, 1.0 / bh as f32],
            offset: 2.0,
            _pad: 0.0,
        };
        self.run_blur_pass(&down_srv, &blur_rtv, bw, bh, &self.ps_up, &cb2);

        drop(down_tex);
        drop(down_srv);
        drop(down_rtv);
        drop(blur_rtv);

        self.sharp_tex = Some((w, h, sharp_tex, sharp_srv));
        self.blurred_tex = Some((bw, bh, blur_tex, blur_srv));
        true
    }

    /// Upload a pre-blurred monitor wallpaper and optional sharp monitor wallpaper directly.
    pub unsafe fn set_wallpaper_pair_bgra(
        &mut self,
        w: i32,
        h: i32,
        blurred_bgra: &[u8],
        sharp_bgra: Option<&[u8]>,
    ) -> bool {
        if w <= 0 || h <= 0 {
            return false;
        }
        let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if blurred_bgra.len() < expected {
            return false;
        }
        let sharp_slice = sharp_bgra.unwrap_or(blurred_bgra);
        if sharp_slice.len() < expected {
            return false;
        }
        let Some((blur_tex, blur_srv)) = create_texture2d(
            &self.device,
            w,
            h,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            Some(blurred_bgra),
            false,
        ) else {
            return false;
        };
        let Some((sharp_tex, sharp_srv)) = create_texture2d(
            &self.device,
            w,
            h,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            Some(sharp_slice),
            false,
        ) else {
            return false;
        };
        self.blurred_tex = Some((w, h, blur_tex, blur_srv));
        self.sharp_tex = Some((w, h, sharp_tex, sharp_srv));
        true
    }

    unsafe fn run_blur_pass(
        &self,
        src_srv: &ComPtr,
        dst_rtv: &ComPtr,
        w: i32,
        h: i32,
        ps: &ComPtr,
        cb: &BlurConstants,
    ) {
        update_subresource(&self.context, &self.blur_cb, cb as *const _ as *const c_void);
        set_viewport(&self.context, w as f32, h as f32);
        om_set_render_targets(&self.context, dst_rtv.as_ptr());
        ia_set_topology(&self.context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        vs_set_shader(&self.context, self.vs.as_ptr());
        ps_set_shader(&self.context, ps.as_ptr());
        ps_set_cb(&self.context, 1, self.blur_cb.as_ptr());
        ps_set_srv(&self.context, 0, src_srv.as_ptr());
        ps_set_sampler(&self.context, 0, self.samp_clamp.as_ptr());
        draw(&self.context, 3, 0);
        ps_set_srv(&self.context, 0, null_mut());
        om_set_render_targets(&self.context, null_mut());
    }

    /// Render the unified glass + SDF + liquid glass + nested glass + optional
    /// content texture into `dst_rtv`.
    pub unsafe fn render_glass(
        &self,
        dst_rtv: &ComPtr,
        w: i32,
        h: i32,
        constants: &GlassConstants,
        content_srv: Option<&ComPtr>,
    ) {
        let mut cb = *constants;
        cb.has_content = if content_srv.is_some() { 1 } else { 0 };
        update_subresource(&self.context, &self.glass_cb, &cb as *const _ as *const c_void);

        let clear = [0.0f32, 0.0, 0.0, 0.0];
        clear_rtv(&self.context, dst_rtv.as_ptr(), &clear);
        set_viewport(&self.context, w as f32, h as f32);
        om_set_render_targets(&self.context, dst_rtv.as_ptr());
        ia_set_topology(&self.context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        vs_set_shader(&self.context, self.vs.as_ptr());
        ps_set_shader(&self.context, self.ps_glass.as_ptr());
        ps_set_cb(&self.context, 0, self.glass_cb.as_ptr());
        ps_set_sampler(&self.context, 0, self.samp_clamp.as_ptr());
        ps_set_sampler(&self.context, 1, self.samp_wrap.as_ptr());

        if let Some((_, _, _, srv)) = &self.sharp_tex {
            ps_set_srv(&self.context, 0, srv.as_ptr());
        }
        if let Some((_, _, _, srv)) = &self.blurred_tex {
            ps_set_srv(&self.context, 1, srv.as_ptr());
        }
        if let Some(csrv) = content_srv {
            ps_set_srv(&self.context, 2, csrv.as_ptr());
        }
        ps_set_srv(&self.context, 3, self.grain_srv.as_ptr());
        draw(&self.context, 3, 0);

        ps_set_srv(&self.context, 0, null_mut());
        ps_set_srv(&self.context, 1, null_mut());
        ps_set_srv(&self.context, 2, null_mut());
        om_set_render_targets(&self.context, null_mut());
    }

    /// Render the fused junction bridge (contour SDF / optional smin liquid morph,
    /// member window cutout, liquid rim refraction/CA, and tanh cluster shadow).
    pub unsafe fn render_fusion_bridge(
        &self,
        dst_rtv: &ComPtr,
        w: i32,
        h: i32,
        constants: &FusionBridgeConstants,
        field_srv: Option<&ComPtr>,
    ) {
        update_subresource(
            &self.context,
            &self.fusion_cb,
            constants as *const _ as *const c_void,
        );

        let clear = [0.0f32, 0.0, 0.0, 0.0];
        clear_rtv(&self.context, dst_rtv.as_ptr(), &clear);
        set_viewport(&self.context, w as f32, h as f32);
        om_set_render_targets(&self.context, dst_rtv.as_ptr());
        ia_set_topology(&self.context, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        vs_set_shader(&self.context, self.vs.as_ptr());
        ps_set_shader(&self.context, self.ps_fusion.as_ptr());
        ps_set_cb(&self.context, 2, self.fusion_cb.as_ptr());
        ps_set_sampler(&self.context, 0, self.samp_clamp.as_ptr());
        ps_set_sampler(&self.context, 1, self.samp_wrap.as_ptr());

        if let Some((_, _, _, srv)) = &self.sharp_tex {
            ps_set_srv(&self.context, 0, srv.as_ptr());
        }
        if let Some((_, _, _, srv)) = &self.blurred_tex {
            ps_set_srv(&self.context, 1, srv.as_ptr());
        }
        if let Some(fsrv) = field_srv {
            ps_set_srv(&self.context, 2, fsrv.as_ptr());
        } else {
            ps_set_srv(&self.context, 2, null_mut());
        }
        draw(&self.context, 3, 0);

        ps_set_srv(&self.context, 0, null_mut());
        ps_set_srv(&self.context, 1, null_mut());
        ps_set_srv(&self.context, 2, null_mut());
        om_set_render_targets(&self.context, null_mut());
    }
}

/// Per-widget DirectComposition surface with bucketed swapchain sizing
/// (`IDXGISwapChain2::SetSourceSize`) for zero-reallocation 120Hz resize.
pub struct GpuSurface {
    pub swapchain: ComPtr,
    #[cfg_attr(not(test), allow(dead_code))]
    pub swapchain2: Option<ComPtr>,
    #[allow(dead_code)]
    pub target: ComPtr,
    #[allow(dead_code)]
    pub visual: ComPtr,
    pub rtv: ComPtr,
    pub alloc_w: i32,
    pub alloc_h: i32,
    pub source_w: i32,
    pub source_h: i32,
    content_tex: Option<(i32, i32, ComPtr, ComPtr)>,
    bridge_field_tex: Option<(u32, u32, ComPtr, ComPtr)>,
}

fn bucket_dim(px: i32) -> i32 {
    let px = px.max(64);
    ((px + 127) / 128) * 128
}

impl GpuSurface {
    pub unsafe fn new(gpu: &GpuDevice, hwnd: HWND, w: i32, h: i32) -> Option<Self> {
        let alloc_w = bucket_dim(w);
        let alloc_h = bucket_dim(h);

        let desc = DxgiSwapChainDesc1 {
            width: alloc_w as u32,
            height: alloc_h as u32,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            stereo: 0,
            sample_count: 1,
            sample_quality: 0,
            buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            buffer_count: 3,
            scaling: DXGI_SCALING_STRETCH,
            swap_effect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            alpha_mode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            flags: 0,
        };

        let create_sc: extern "system" fn(
            *mut c_void,
            *mut c_void,
            *const DxgiSwapChainDesc1,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*gpu.dxgi_factory.vtable().add(24));
        let mut sc_ptr = null_mut();
        let hr_sc = create_sc(
            gpu.dxgi_factory.as_ptr(),
            gpu.device.as_ptr(),
            &desc,
            null_mut(),
            &mut sc_ptr,
        );
        if hr_sc < 0 || sc_ptr.is_null() {
            eprintln!("gpu::GpuSurface::new create_sc failed hr=0x{:08x}", hr_sc as u32);
            return None;
        }
        let swapchain = ComPtr(sc_ptr);
        let swapchain2 = swapchain.query_interface(&IID_IDXGISWAPCHAIN2);

        let create_target: extern "system" fn(
            *mut c_void,
            HWND,
            BOOL,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*gpu.dcomp.vtable().add(6));
        let mut target_ptr = null_mut();
        let hr_target = create_target(gpu.dcomp.as_ptr(), hwnd, 1, &mut target_ptr);
        if hr_target < 0 || target_ptr.is_null() {
            eprintln!("gpu::GpuSurface::new create_target failed hr=0x{:08x}", hr_target as u32);
            return None;
        }
        let target = ComPtr(target_ptr);

        let create_visual: extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT =
            std::mem::transmute(*gpu.dcomp.vtable().add(7));
        let mut visual_ptr = null_mut();
        let hr_vis = create_visual(gpu.dcomp.as_ptr(), &mut visual_ptr);
        if hr_vis < 0 || visual_ptr.is_null() {
            eprintln!("gpu::GpuSurface::new create_visual failed hr=0x{:08x}", hr_vis as u32);
            return None;
        }
        let visual = ComPtr(visual_ptr);

        let set_content: extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
            std::mem::transmute(*visual.vtable().add(15));
        let _ = set_content(visual.as_ptr(), swapchain.as_ptr());

        let set_root: extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
            std::mem::transmute(*target.vtable().add(3));
        let _ = set_root(target.as_ptr(), visual.as_ptr());

        let rtv = swapchain_rtv(&gpu.device, &swapchain)?;
        let mut s = Self {
            swapchain,
            swapchain2,
            target,
            visual,
            rtv,
            alloc_w,
            alloc_h,
            source_w: w.max(1),
            source_h: h.max(1),
            content_tex: None,
            bridge_field_tex: None,
        };
        s.resize(gpu, w, h);
        Some(s)
    }

    /// Resize without reallocating the swapchain unless the window exceeds the
    /// current 128px bucket.
    pub unsafe fn resize(&mut self, gpu: &GpuDevice, w: i32, h: i32) -> bool {
        let w = w.max(1);
        let h = h.max(1);
        let mut reallocated = false;
        if w > self.alloc_w || h > self.alloc_h {
            let new_w = bucket_dim(w);
            let new_h = bucket_dim(h);
            // Drop RTV before ResizeBuffers
            let dummy = ComPtr(null_mut());
            let old = std::mem::replace(&mut self.rtv, dummy);
            drop(old);
            let resize_buf: extern "system" fn(
                *mut c_void,
                u32,
                u32,
                u32,
                u32,
                u32,
            ) -> HRESULT = std::mem::transmute(*self.swapchain.vtable().add(13));
            if resize_buf(
                self.swapchain.as_ptr(),
                0,
                new_w as u32,
                new_h as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                0,
            ) >= 0
            {
                if let Some(rtv) = swapchain_rtv(&gpu.device, &self.swapchain) {
                    self.rtv = rtv;
                    self.alloc_w = new_w;
                    self.alloc_h = new_h;
                    reallocated = true;
                }
            }
        }
        // Keep full backbuffer source size; CreateSwapChainForComposition uses DXGI_SCALING_STRETCH
        // so calling SetSourceSize(w, h) without an inverse visual scale zooms [0..w, 0..h]
        // up to (alloc_w, alloc_h) and crops right/bottom widget edges. Rendering with a (w, h)
        // viewport over a cleared backbuffer keeps exact 1:1 pixels with zero reallocation.
        self.source_w = w;
        self.source_h = h;
        reallocated
    }

    pub unsafe fn upload_content(&mut self, gpu: &GpuDevice, w: i32, h: i32, bgra: &[u8]) {
        if w <= 0 || h <= 0 || bgra.len() < (w * h * 4) as usize {
            return;
        }
        let need_new = self
            .content_tex
            .as_ref()
            .is_none_or(|(cw, ch, _, _)| w > *cw || h > *ch);
        if need_new {
            let bw = bucket_dim(w);
            let bh = bucket_dim(h);
            self.content_tex =
                create_texture2d(&gpu.device, bw, bh, DXGI_FORMAT_B8G8R8A8_UNORM, None, true)
                    .map(|(t, s)| (bw, bh, t, s));
        }
        if let Some((_, _, tex, _)) = &self.content_tex {
            let map: extern "system" fn(
                *mut c_void,
                *mut c_void,
                u32,
                u32,
                u32,
                *mut D3D11MappedSubresource,
            ) -> HRESULT = std::mem::transmute(*gpu.context.vtable().add(14));
            let unmap: extern "system" fn(*mut c_void, *mut c_void, u32) =
                std::mem::transmute(*gpu.context.vtable().add(15));
            let mut mapped: D3D11MappedSubresource = zeroed();
            if map(
                gpu.context.as_ptr(),
                tex.as_ptr(),
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            ) >= 0
                && !mapped.p_data.is_null()
            {
                let row_bytes = (w * 4) as usize;
                let dst = mapped.p_data as *mut u8;
                for y in 0..h as usize {
                    std::ptr::copy_nonoverlapping(
                        bgra.as_ptr().add(y * row_bytes),
                        dst.add(y * mapped.row_pitch as usize),
                        row_bytes,
                    );
                }
                unmap(gpu.context.as_ptr(), tex.as_ptr(), 0);
            }
        }
    }

    pub unsafe fn present(&self, gpu: &GpuDevice, constants: &GlassConstants) {
        let mut cb = *constants;
        cb.content_uv_scale = if let Some((cw, ch, _, _)) = &self.content_tex {
            let w = self.source_w as f32;
            let h = self.source_h as f32;
            let cw = (*cw).max(1) as f32;
            let ch = (*ch).max(1) as f32;
            [
                w / cw,
                h / ch,
                (w - 0.5).max(0.5) / cw,
                (h - 0.5).max(0.5) / ch,
            ]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let csrv = self.content_tex.as_ref().map(|(_, _, _, s)| s);
        gpu.render_glass(&self.rtv, self.source_w, self.source_h, &cb, csrv);
        let present: extern "system" fn(*mut c_void, u32, u32) -> HRESULT =
            std::mem::transmute(*self.swapchain.vtable().add(8));
        let _ = present(self.swapchain.as_ptr(), 0, 0);
        let commit: extern "system" fn(*mut c_void) -> HRESULT =
            std::mem::transmute(*gpu.dcomp.vtable().add(3));
        let _ = commit(gpu.dcomp.as_ptr());
    }

    pub unsafe fn upload_bridge_field(
        &mut self,
        gpu: &GpuDevice,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        if width == 0 || height == 0 || rgba.len() < (width as usize).saturating_mul(height as usize).saturating_mul(4) {
            return;
        }
        let need_new = self
            .bridge_field_tex
            .as_ref()
            .is_none_or(|(fw, fh, _, _)| *fw != width || *fh != height);
        if need_new {
            self.bridge_field_tex = create_texture2d(
                &gpu.device,
                width as i32,
                height as i32,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                None,
                true,
            )
            .map(|(t, s)| (width, height, t, s));
        }
        if let Some((_, _, tex, _)) = &self.bridge_field_tex {
            let map: extern "system" fn(
                *mut c_void,
                *mut c_void,
                u32,
                u32,
                u32,
                *mut D3D11MappedSubresource,
            ) -> HRESULT = std::mem::transmute(*gpu.context.vtable().add(14));
            let unmap: extern "system" fn(*mut c_void, *mut c_void, u32) =
                std::mem::transmute(*gpu.context.vtable().add(15));
            let mut mapped: D3D11MappedSubresource = zeroed();
            if map(
                gpu.context.as_ptr(),
                tex.as_ptr(),
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            ) >= 0
                && !mapped.p_data.is_null()
            {
                let row_bytes = (width * 4) as usize;
                let dst = mapped.p_data as *mut u8;
                for y in 0..height as usize {
                    std::ptr::copy_nonoverlapping(
                        rgba.as_ptr().add(y * row_bytes),
                        dst.add(y * mapped.row_pitch as usize),
                        row_bytes,
                    );
                }
                unmap(gpu.context.as_ptr(), tex.as_ptr(), 0);
            }
        }
    }

    pub unsafe fn present_fusion(&self, gpu: &GpuDevice, constants: &FusionBridgeConstants) {
        let field_srv = (constants.field_uv[0] > 0.0)
            .then(|| self.bridge_field_tex.as_ref().map(|(_, _, _, s)| s))
            .flatten();
        gpu.render_fusion_bridge(&self.rtv, self.source_w, self.source_h, constants, field_srv);
        let present: extern "system" fn(*mut c_void, u32, u32) -> HRESULT =
            std::mem::transmute(*self.swapchain.vtable().add(8));
        let _ = present(self.swapchain.as_ptr(), 0, 0);
        let commit: extern "system" fn(*mut c_void) -> HRESULT =
            std::mem::transmute(*gpu.dcomp.vtable().add(3));
        let _ = commit(gpu.dcomp.as_ptr());
    }
}

struct GpuCompositor {
    device: Option<GpuDevice>,
    surfaces: std::collections::HashMap<isize, GpuSurface>,
    wallpaper_key: Option<(u64, [i32; 4])>,
}

thread_local! {
    static GPU_COMPOSITOR: std::cell::RefCell<Option<GpuCompositor>> =
        const { std::cell::RefCell::new(None) };
}

fn with_compositor<R>(f: impl FnOnce(&mut GpuCompositor) -> R) -> R {
    GPU_COMPOSITOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        let comp = slot.get_or_insert_with(|| {
            // Widget backgrounds are still rasterized on the CPU. Enabling
            // composition adds uploads/presents rather than replacing that
            // work, so keep this path opt-in until it is measured end-to-end.
            let force_cpu = std::env::var_os("LUMEN_FORCE_CPU").is_some()
                || std::env::var("LUMEN_GPU_COMPOSITOR").as_deref() != Ok("1");
            GpuCompositor {
                device: if force_cpu { None } else { GpuDevice::new() },
                surfaces: std::collections::HashMap::new(),
                wallpaper_key: None,
            }
        });
        f(comp)
    })
}

pub fn available() -> bool {
    with_compositor(|c| c.device.is_some())
}

pub fn window_ex_style() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP};
    if !cfg!(test) && available() {
        WS_EX_NOREDIRECTIONBITMAP
    } else {
        WS_EX_LAYERED
    }
}

pub unsafe fn is_gpu_window(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GWL_EXSTYLE, WS_EX_NOREDIRECTIONBITMAP,
    };
    (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP) != 0
}

pub unsafe fn present_bgra(hwnd: HWND, w: i32, h: i32, bgra: &[u8]) -> bool {
    with_compositor(|comp| {
        let Some(gpu) = comp.device.as_ref() else {
            return false;
        };
        let key = hwnd as isize;
        if !comp.surfaces.contains_key(&key) {
            let Some(surface) = GpuSurface::new(gpu, hwnd, w, h) else {
                return false;
            };
            comp.surfaces.insert(key, surface);
        }
        let Some(surface) = comp.surfaces.get_mut(&key) else {
            return false;
        };
        if surface.source_w != w.max(1) || surface.source_h != h.max(1) {
            surface.resize(gpu, w, h);
        }
        surface.upload_content(gpu, w, h, bgra);
        let constants = GlassConstants {
            win_rect: [0.0, 0.0, w as f32, h as f32],
            screen_size: [w as f32, h as f32],
            radius: 0.0,
            dpi: 1.0,
            tint_color: [0.0; 4],
            flags: [0.0; 4],
            nested_count: 0,
            has_content: 1,
            fuse_line_count: 0,
            fuse_corner_count: 0,
            content_uv_scale: [1.0, 1.0, 1.0, 1.0],
            nested: [NestedGlassRect::default(); 8],
            fuse_lines: [[0.0; 4]; 32],
            fuse_corner_c: [[0.0; 4]; 32],
            fuse_corner_uv: [[0.0; 4]; 32],
        };
        surface.present(gpu, &constants);
        true
    })
}

pub(crate) fn wallpaper_revision_synced(revision: u64, area: [i32; 4]) -> bool {
    with_compositor(|comp| {
        comp.wallpaper_key == Some((revision, area))
            && comp
                .device
                .as_ref()
                .is_some_and(|g| g.sharp_tex.is_some() && g.blurred_tex.is_some())
    })
}

pub unsafe fn sync_wallpaper_pair(
    revision: u64,
    area: [i32; 4],
    blurred: &[u8],
    sharp: Option<&[u8]>,
) -> bool {
    with_compositor(|comp| {
        if comp.wallpaper_key == Some((revision, area))
            && comp
                .device
                .as_ref()
                .is_some_and(|g| g.sharp_tex.is_some() && g.blurred_tex.is_some())
        {
            return true;
        }
        let Some(gpu) = comp.device.as_mut() else {
            return false;
        };
        if gpu.set_wallpaper_pair_bgra(area[2], area[3], blurred, sharp) {
            comp.wallpaper_key = Some((revision, area));
            true
        } else {
            false
        }
    })
}

#[allow(dead_code)]
pub unsafe fn sync_wallpaper_bgra(revision: u64, w: i32, h: i32, bgra: &[u8]) -> bool {
    with_compositor(|comp| {
        let area = [0, 0, w, h];
        if comp.wallpaper_key == Some((revision, area))
            && comp.device.as_ref().is_some_and(|g| g.sharp_tex.is_some())
        {
            return true;
        }
        let Some(gpu) = comp.device.as_mut() else {
            return false;
        };
        if gpu.set_wallpaper_bgra(w, h, bgra) {
            comp.wallpaper_key = Some((revision, area));
            true
        } else {
            false
        }
    })
}

pub unsafe fn present_fusion_bridge(hwnd: HWND, constants: &FusionBridgeConstants) -> bool {
    present_fusion_bridge_with_field(hwnd, constants, None)
}

pub unsafe fn present_fusion_bridge_with_field(
    hwnd: HWND,
    constants: &FusionBridgeConstants,
    field: Option<(u32, u32, &[u8])>,
) -> bool {
    let w = constants.bridge_rect[2].round().max(1.0) as i32;
    let h = constants.bridge_rect[3].round().max(1.0) as i32;
    with_compositor(|comp| {
        let Some(gpu) = comp.device.as_ref() else {
            return false;
        };
        let key = hwnd as isize;
        if !comp.surfaces.contains_key(&key) {
            let Some(surface) = GpuSurface::new(gpu, hwnd, w, h) else {
                return false;
            };
            comp.surfaces.insert(key, surface);
        }
        let Some(surface) = comp.surfaces.get_mut(&key) else {
            return false;
        };
        if surface.source_w != w || surface.source_h != h {
            surface.resize(gpu, w, h);
        }
        if let Some((fw, fh, rgba)) = field {
            surface.upload_bridge_field(gpu, fw, fh, rgba);
        }
        surface.present_fusion(gpu, constants);
        true
    })
}

pub fn remove_surface(hwnd: HWND) {
    GPU_COMPOSITOR.with(|cell| {
        if let Some(comp) = cell.borrow_mut().as_mut() {
            comp.surfaces.remove(&(hwnd as isize));
        }
    });
}

/// Release D3D11/DXGI/DirectComposition resources before process TLS teardown.
pub fn shutdown() {
    GPU_COMPOSITOR.with(|cell| {
        if let Some(mut comp) = cell.borrow_mut().take() {
            comp.surfaces.clear();
            comp.device.take();
        }
    });
}

#[repr(C)]
struct D3dShaderMacro {
    name: *const u8,
    definition: *const u8,
}

unsafe fn compile_shader(entry: &str, target: &str) -> Option<ComPtr> {
    let fusion_macros = [
        D3dShaderMacro {
            name: b"FUSION_BRIDGE_PS\0".as_ptr(),
            definition: b"1\0".as_ptr(),
        },
        D3dShaderMacro {
            name: null(),
            definition: null(),
        },
    ];
    let p_defines = if entry == "fusion_bridge_ps\0" {
        fusion_macros.as_ptr() as *const c_void
    } else {
        null()
    };
    let mut blob = null_mut();
    let mut errors = null_mut();
    let hr = D3DCompile(
        SHADER_SRC.as_ptr(),
        SHADER_SRC.len(),
        b"lumen_glass.hlsl\0".as_ptr(),
        p_defines,
        null(),
        entry.as_ptr(),
        target.as_ptr(),
        1 << 15, // D3DCOMPILE_OPTIMIZATION_LEVEL3
        0,
        &mut blob,
        &mut errors,
    );
    let errors = (!errors.is_null()).then(|| ComPtr(errors));
    if hr >= 0 && !blob.is_null() {
        Some(ComPtr(blob))
    } else {
        let detail = errors
            .as_ref()
            .map(|errors| {
                let (ptr, len) = blob_bytes(errors);
                String::from_utf8_lossy(std::slice::from_raw_parts(ptr as *const u8, len))
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .unwrap_or_default();
        let message = format!(
            "shader {} failed hr=0x{:08x}: {}",
            entry.trim_end_matches('\0'),
            hr as u32,
            detail
        );
        crate::logger::error("gpu", &message);
        eprintln!("{message}");
        if !blob.is_null() {
            drop(ComPtr(blob));
        }
        None
    }
}

unsafe fn blob_bytes(blob: &ComPtr) -> (*const c_void, usize) {
    let ptr_fn: extern "system" fn(*mut c_void) -> *const c_void =
        std::mem::transmute(*blob.vtable().add(3));
    let size_fn: extern "system" fn(*mut c_void) -> usize =
        std::mem::transmute(*blob.vtable().add(4));
    (ptr_fn(blob.as_ptr()), size_fn(blob.as_ptr()))
}

unsafe fn create_vs(device: &ComPtr, blob: &ComPtr) -> Option<ComPtr> {
    let (ptr, len) = blob_bytes(blob);
    let f: extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(12));
    let mut out = null_mut();
    (f(device.as_ptr(), ptr, len, null_mut(), &mut out) >= 0 && !out.is_null())
        .then(|| ComPtr(out))
}

unsafe fn create_ps(device: &ComPtr, blob: &ComPtr) -> Option<ComPtr> {
    let (ptr, len) = blob_bytes(blob);
    let f: extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(15));
    let mut out = null_mut();
    (f(device.as_ptr(), ptr, len, null_mut(), &mut out) >= 0 && !out.is_null())
        .then(|| ComPtr(out))
}

unsafe fn create_sampler(device: &ComPtr, addr: u32) -> Option<ComPtr> {
    let desc = D3D11SamplerDesc {
        filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        address_u: addr,
        address_v: addr,
        address_w: addr,
        mip_lod_bias: 0.0,
        max_anisotropy: 1,
        comparison_func: 0,
        border_color: [0.0; 4],
        min_lod: 0.0,
        max_lod: f32::MAX,
    };
    let f: extern "system" fn(*mut c_void, *const D3D11SamplerDesc, *mut *mut c_void) -> HRESULT =
        std::mem::transmute(*device.vtable().add(23));
    let mut out = null_mut();
    (f(device.as_ptr(), &desc, &mut out) >= 0 && !out.is_null()).then(|| ComPtr(out))
}

unsafe fn create_cb(device: &ComPtr, size: u32) -> Option<ComPtr> {
    let aligned = (size + 15) & !15;
    let desc = D3D11BufferDesc {
        byte_width: aligned,
        usage: D3D11_USAGE_DEFAULT,
        bind_flags: D3D11_BIND_CONSTANT_BUFFER,
        cpu_access_flags: 0,
        misc_flags: 0,
        structure_byte_stride: 0,
    };
    let f: extern "system" fn(
        *mut c_void,
        *const D3D11BufferDesc,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(3));
    let mut out = null_mut();
    (f(device.as_ptr(), &desc, null(), &mut out) >= 0 && !out.is_null()).then(|| ComPtr(out))
}

unsafe fn create_texture2d(
    device: &ComPtr,
    w: i32,
    h: i32,
    format: u32,
    data: Option<&[u8]>,
    dynamic: bool,
) -> Option<(ComPtr, ComPtr)> {
    let desc = D3D11Texture2dDesc {
        width: w as u32,
        height: h as u32,
        mip_levels: 1,
        array_size: 1,
        format,
        sample_count: 1,
        sample_quality: 0,
        usage: if dynamic {
            D3D11_USAGE_DYNAMIC
        } else {
            D3D11_USAGE_DEFAULT
        },
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        cpu_access_flags: if dynamic { D3D11_CPU_ACCESS_WRITE } else { 0 },
        misc_flags: 0,
    };
    let init = data.map(|bytes| D3D11SubresourceData {
        p_sys_mem: bytes.as_ptr() as *const c_void,
        sys_mem_pitch: (w * 4) as u32,
        sys_mem_slice_pitch: 0,
    });
    let f: extern "system" fn(
        *mut c_void,
        *const D3D11Texture2dDesc,
        *const D3D11SubresourceData,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(5));
    let mut tex_ptr = null_mut();
    if f(
        device.as_ptr(),
        &desc,
        init.as_ref().map_or(null(), |d| d as *const _),
        &mut tex_ptr,
    ) < 0
        || tex_ptr.is_null()
    {
        return None;
    }
    let tex = ComPtr(tex_ptr);
    let srv = create_srv(device, &tex)?;
    Some((tex, srv))
}

unsafe fn create_rt_texture(device: &ComPtr, w: i32, h: i32) -> Option<(ComPtr, ComPtr, ComPtr)> {
    let desc = D3D11Texture2dDesc {
        width: w as u32,
        height: h as u32,
        mip_levels: 1,
        array_size: 1,
        format: DXGI_FORMAT_B8G8R8A8_UNORM,
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_DEFAULT,
        bind_flags: D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET,
        cpu_access_flags: 0,
        misc_flags: 0,
    };
    let f: extern "system" fn(
        *mut c_void,
        *const D3D11Texture2dDesc,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(5));
    let mut tex_ptr = null_mut();
    if f(device.as_ptr(), &desc, null(), &mut tex_ptr) < 0 || tex_ptr.is_null() {
        return None;
    }
    let tex = ComPtr(tex_ptr);
    let srv = create_srv(device, &tex)?;
    let rtv = create_rtv(device, &tex)?;
    Some((tex, srv, rtv))
}

unsafe fn create_srv(device: &ComPtr, res: &ComPtr) -> Option<ComPtr> {
    let f: extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(7));
    let mut out = null_mut();
    (f(device.as_ptr(), res.as_ptr(), null(), &mut out) >= 0 && !out.is_null())
        .then(|| ComPtr(out))
}

unsafe fn create_rtv(device: &ComPtr, res: &ComPtr) -> Option<ComPtr> {
    let f: extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT = std::mem::transmute(*device.vtable().add(9));
    let mut out = null_mut();
    (f(device.as_ptr(), res.as_ptr(), null(), &mut out) >= 0 && !out.is_null())
        .then(|| ComPtr(out))
}

unsafe fn swapchain_rtv(device: &ComPtr, swapchain: &ComPtr) -> Option<ComPtr> {
    let get_buf: extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT =
        std::mem::transmute(*swapchain.vtable().add(9));
    let mut tex_ptr = null_mut();
    if get_buf(swapchain.as_ptr(), 0, &IID_ID3D11TEXTURE2D, &mut tex_ptr) < 0 || tex_ptr.is_null()
    {
        return None;
    }
    let tex = ComPtr(tex_ptr);
    create_rtv(device, &tex)
}

unsafe fn create_grain_srv(device: &ComPtr) -> Option<ComPtr> {
    let mut pixels = vec![0u8; 128 * 128 * 4];
    let mut seed: u32 = 0x12345678;
    for p in pixels.chunks_exact_mut(4) {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let v = (seed >> 24) as u8;
        p[0] = v;
        p[1] = v;
        p[2] = v;
        p[3] = 255;
    }
    let (_tex, srv) =
        create_texture2d(device, 128, 128, DXGI_FORMAT_R8G8B8A8_UNORM, Some(&pixels), false)?;
    Some(srv)
}

unsafe fn update_subresource(ctx: &ComPtr, res: &ComPtr, data: *const c_void) {
    let f: extern "system" fn(*mut c_void, *mut c_void, u32, *const c_void, *const c_void, u32, u32) =
        std::mem::transmute(*ctx.vtable().add(48));
    f(ctx.as_ptr(), res.as_ptr(), 0, null(), data, 0, 0);
}

unsafe fn set_viewport(ctx: &ComPtr, w: f32, h: f32) {
    let vp = D3D11Viewport {
        top_left_x: 0.0,
        top_left_y: 0.0,
        width: w,
        height: h,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let f: extern "system" fn(*mut c_void, u32, *const D3D11Viewport) =
        std::mem::transmute(*ctx.vtable().add(44));
    f(ctx.as_ptr(), 1, &vp);
}

unsafe fn clear_rtv(ctx: &ComPtr, rtv: *mut c_void, rgba: &[f32; 4]) {
    let f: extern "system" fn(*mut c_void, *mut c_void, *const f32) =
        std::mem::transmute(*ctx.vtable().add(50));
    f(ctx.as_ptr(), rtv, rgba.as_ptr());
}

unsafe fn om_set_render_targets(ctx: &ComPtr, rtv: *mut c_void) {
    let f: extern "system" fn(*mut c_void, u32, *const *mut c_void, *mut c_void) =
        std::mem::transmute(*ctx.vtable().add(33));
    if rtv.is_null() {
        f(ctx.as_ptr(), 0, null(), null_mut());
    } else {
        f(ctx.as_ptr(), 1, &rtv, null_mut());
    }
}

unsafe fn ia_set_topology(ctx: &ComPtr, topo: u32) {
    let f: extern "system" fn(*mut c_void, u32) = std::mem::transmute(*ctx.vtable().add(24));
    f(ctx.as_ptr(), topo);
}

unsafe fn vs_set_shader(ctx: &ComPtr, vs: *mut c_void) {
    let f: extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32) =
        std::mem::transmute(*ctx.vtable().add(11));
    f(ctx.as_ptr(), vs, null(), 0);
}

unsafe fn ps_set_shader(ctx: &ComPtr, ps: *mut c_void) {
    let f: extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32) =
        std::mem::transmute(*ctx.vtable().add(9));
    f(ctx.as_ptr(), ps, null(), 0);
}

unsafe fn ps_set_cb(ctx: &ComPtr, slot: u32, cb: *mut c_void) {
    let f: extern "system" fn(*mut c_void, u32, u32, *const *mut c_void) =
        std::mem::transmute(*ctx.vtable().add(16));
    f(ctx.as_ptr(), slot, 1, &cb);
}

unsafe fn ps_set_srv(ctx: &ComPtr, slot: u32, srv: *mut c_void) {
    let f: extern "system" fn(*mut c_void, u32, u32, *const *mut c_void) =
        std::mem::transmute(*ctx.vtable().add(8));
    f(ctx.as_ptr(), slot, 1, &srv);
}

unsafe fn ps_set_sampler(ctx: &ComPtr, slot: u32, samp: *mut c_void) {
    let f: extern "system" fn(*mut c_void, u32, u32, *const *mut c_void) =
        std::mem::transmute(*ctx.vtable().add(10));
    f(ctx.as_ptr(), slot, 1, &samp);
}

unsafe fn draw(ctx: &ComPtr, count: u32, start: u32) {
    let f: extern "system" fn(*mut c_void, u32, u32) = std::mem::transmute(*ctx.vtable().add(13));
    f(ctx.as_ptr(), count, start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WNDCLASSW,
        WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    #[test]
    fn all_shader_entry_points_compile_without_a_gpu() {
        // Do not put this behind GpuDevice::new(): a shader failure used to
        // disable the compositor and silently skip the GPU integration tests.
        for (entry, target) in [
            ("fullscreen_vs\0", "vs_5_0\0"),
            ("kawase_down_ps\0", "ps_5_0\0"),
            ("kawase_up_ps\0", "ps_5_0\0"),
            ("glass_composite_ps\0", "ps_5_0\0"),
            ("fusion_bridge_ps\0", "ps_5_0\0"),
        ] {
            assert!(
                unsafe { compile_shader(entry, target) }.is_some(),
                "{entry:?}"
            );
        }
    }

    unsafe fn read_rt_pixel(gpu: &GpuDevice, tex: &ComPtr, w: i32, h: i32, x: i32, y: i32) -> [u8; 4] {
        let desc = D3D11Texture2dDesc {
            width: w as u32,
            height: h as u32,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            sample_count: 1,
            sample_quality: 0,
            usage: D3D11_USAGE_STAGING,
            bind_flags: 0,
            cpu_access_flags: D3D11_CPU_ACCESS_READ,
            misc_flags: 0,
        };
        let create_tex: extern "system" fn(
            *mut c_void,
            *const D3D11Texture2dDesc,
            *const c_void,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(*gpu.device.vtable().add(5));
        let mut stg_ptr = null_mut();
        assert!(create_tex(gpu.device.as_ptr(), &desc, null(), &mut stg_ptr) >= 0);
        let stg = ComPtr(stg_ptr);

        let copy_res: extern "system" fn(*mut c_void, *mut c_void, *mut c_void) =
            std::mem::transmute(*gpu.context.vtable().add(47));
        copy_res(gpu.context.as_ptr(), stg.as_ptr(), tex.as_ptr());

        let map: extern "system" fn(
            *mut c_void,
            *mut c_void,
            u32,
            u32,
            u32,
            *mut D3D11MappedSubresource,
        ) -> HRESULT = std::mem::transmute(*gpu.context.vtable().add(14));
        let unmap: extern "system" fn(*mut c_void, *mut c_void, u32) =
            std::mem::transmute(*gpu.context.vtable().add(15));
        let mut mapped: D3D11MappedSubresource = zeroed();
        assert!(map(gpu.context.as_ptr(), stg.as_ptr(), 0, D3D11_MAP_READ, 0, &mut mapped) >= 0);
        let ptr = (mapped.p_data as *const u8).add((y * mapped.row_pitch as i32 + x * 4) as usize);
        let px = [*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)];
        unmap(gpu.context.as_ptr(), stg.as_ptr(), 0);
        px
    }

    #[test]
    fn gpu_compositor_blur_sdf_liquid_nested_and_bucketed_resize() {
        let _ui = crate::native_ui_test_lock();
        let Some(mut gpu) = GpuDevice::new() else {
            // Headless CI without D3D11 hardware device
            return;
        };

        unsafe {
            // 1. Upload shared wallpaper and run GPU Dual Kawase Blur.
            let mut wall = vec![0u8; 256 * 256 * 4];
            for y in 0..256 {
                for x in 0..256 {
                    let i = (y * 256 + x) * 4;
                    wall[i] = (x as u8).wrapping_mul(2);
                    wall[i + 1] = (y as u8).wrapping_mul(2);
                    wall[i + 2] = 180;
                    wall[i + 3] = 255;
                }
            }
            assert!(gpu.set_wallpaper_bgra(256, 256, &wall));

            // 2. Render rounded liquid + frosted + nested glass offscreen.
            let (rt_tex, _rt_srv, rt_rtv) =
                create_rt_texture(&gpu.device, 160, 120).expect("rt texture");
            let mut constants = GlassConstants {
                win_rect: [20.0, 20.0, 160.0, 120.0],
                screen_size: [256.0, 256.0],
                radius: 24.0,
                dpi: 1.0,
                tint_color: [0.15, 0.2, 0.3, 0.45],
                flags: [1.0, 1.0, 0.85, 1.0],
                nested_count: 1,
                has_content: 0,
                fuse_line_count: 0,
                fuse_corner_count: 0,
                content_uv_scale: [1.0, 1.0, 1.0, 1.0],
                nested: [NestedGlassRect::default(); 8],
                fuse_lines: [[0.0; 4]; 32],
                fuse_corner_c: [[0.0; 4]; 32],
                fuse_corner_uv: [[0.0; 4]; 32],
            };
            constants.nested[0] = NestedGlassRect {
                rect: [20.0, 20.0, 140.0, 60.0],
                radius: 16.0,
                tint_alpha: 0.55,
                _pad: [0.0; 2],
            };
            gpu.render_glass(&rt_rtv, 160, 120, &constants, None);

            // Corner (0,0) outside SDF rounded rect must be transparent.
            let corner = read_rt_pixel(&gpu, &rt_tex, 160, 120, 0, 0);
            assert_eq!(corner[3], 0);
            // Center (80,40) inside nested glass must be opaque.
            let center = read_rt_pixel(&gpu, &rt_tex, 160, 120, 80, 40);
            assert!(center[3] >= 250);

            // 3. DirectComposition GpuSurface + bucketed SetSourceSize resize (no realloc within 128px bucket).
            let instance = GetModuleHandleW(null());
            let class_name: Vec<u16> = "LumenGpuTestWindow\0".encode_utf16().collect();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..zeroed()
            };
            let _ = RegisterClassW(&wc);
            let _ = GetLastError();
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                160,
                120,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!hwnd.is_null());
            if let Some(mut surface) = GpuSurface::new(&gpu, hwnd, 160, 120) {
                let assert_source_size = |surface: &GpuSurface| {
                    if let Some(sc2) = &surface.swapchain2 {
                        let get_source_size: extern "system" fn(
                            *mut c_void,
                            *mut u32,
                            *mut u32,
                        ) -> HRESULT = std::mem::transmute(*sc2.vtable().add(30));
                        let (mut sw, mut sh) = (0u32, 0u32);
                        assert!(get_source_size(sc2.as_ptr(), &mut sw, &mut sh) >= 0);
                        assert_eq!((sw as i32, sh as i32), (surface.alloc_w, surface.alloc_h));
                    }
                };
                assert_eq!(surface.alloc_w, 256);
                assert_eq!(surface.alloc_h, 128);
                assert_source_size(&surface);

                let content_160 = vec![0u8; 160 * 120 * 4];
                surface.upload_content(&gpu, 160, 120, &content_160);
                let (cw0, ch0, tex_ptr0) = surface
                    .content_tex
                    .as_ref()
                    .map(|(cw, ch, tex, _)| (*cw, *ch, tex.as_ptr()))
                    .expect("bucketed content_tex");
                assert_eq!((cw0, ch0), (256, 128));

                // Resizing within 256x128 bucket must NOT reallocate swapchain or content_tex.
                assert!(!surface.resize(&gpu, 200, 125));
                assert_eq!(surface.source_w, 200);
                assert_eq!(surface.source_h, 125);
                assert_source_size(&surface);

                let mut content_200 = vec![0u8; 200 * 125 * 4];
                let marker = ((124 * 200) + 199) * 4;
                content_200[marker..marker + 4].copy_from_slice(&[10, 240, 250, 255]);
                surface.upload_content(&gpu, 200, 125, &content_200);
                let (cw1, ch1, tex_ptr1) = surface
                    .content_tex
                    .as_ref()
                    .map(|(cw, ch, tex, _)| (*cw, *ch, tex.as_ptr()))
                    .expect("reused content_tex");
                assert_eq!((cw1, ch1, tex_ptr1), (cw0, ch0, tex_ptr0));

                // Verify 1:1 bucketed pixel mapping without SetSourceSize zoom or right/bottom crop.
                let (bucket_tex, _bucket_rt_srv, bucket_rtv) =
                    create_rt_texture(&gpu.device, 256, 128).expect("bucket rt");
                let (bucket_content, bucket_srv) = create_texture2d(
                    &gpu.device,
                    256,
                    128,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    None,
                    true,
                )
                .expect("bucket content");
                let map: extern "system" fn(
                    *mut c_void,
                    *mut c_void,
                    u32,
                    u32,
                    u32,
                    *mut D3D11MappedSubresource,
                ) -> HRESULT = std::mem::transmute(*gpu.context.vtable().add(14));
                let unmap: extern "system" fn(*mut c_void, *mut c_void, u32) =
                    std::mem::transmute(*gpu.context.vtable().add(15));
                let mut mapped: D3D11MappedSubresource = zeroed();
                assert!(
                    map(
                        gpu.context.as_ptr(),
                        bucket_content.as_ptr(),
                        0,
                        D3D11_MAP_WRITE_DISCARD,
                        0,
                        &mut mapped,
                    ) >= 0
                );
                let row_bytes = 200 * 4;
                let dst = mapped.p_data as *mut u8;
                for y in 0..125usize {
                    std::ptr::copy_nonoverlapping(
                        content_200.as_ptr().add(y * row_bytes),
                        dst.add(y * mapped.row_pitch as usize),
                        row_bytes,
                    );
                }
                unmap(gpu.context.as_ptr(), bucket_content.as_ptr(), 0);
                let cb_200 = GlassConstants {
                    win_rect: [0.0, 0.0, 200.0, 125.0],
                    screen_size: [200.0, 125.0],
                    radius: 0.0,
                    dpi: 1.0,
                    tint_color: [0.0; 4],
                    flags: [0.0; 4],
                    nested_count: 0,
                    has_content: 1,
                    fuse_line_count: 0,
                    fuse_corner_count: 0,
                    content_uv_scale: [200.0 / 256.0, 125.0 / 128.0, 199.5 / 256.0, 124.5 / 128.0],
                    nested: [NestedGlassRect::default(); 8],
                    fuse_lines: [[0.0; 4]; 32],
                    fuse_corner_c: [[0.0; 4]; 32],
                    fuse_corner_uv: [[0.0; 4]; 32],
                };
                gpu.render_glass(&bucket_rtv, 200, 125, &cb_200, Some(&bucket_srv));
                let br_px = read_rt_pixel(&gpu, &bucket_tex, 256, 128, 199, 124);
                assert!(
                    br_px[2] >= 240 && br_px[1] >= 230 && br_px[0] <= 20 && br_px[3] == 255,
                    "bottom-right pixel (199,124) must map 1:1 without crop/stretch: {br_px:?}"
                );
                let outside_px = read_rt_pixel(&gpu, &bucket_tex, 256, 128, 205, 124);
                assert_eq!(outside_px[3], 0, "pixel outside 200x125 viewport must stay transparent");

                // Exceeding 128px height bucket reallocates to 256.
                assert!(surface.resize(&gpu, 200, 140));
                assert_eq!(surface.alloc_h, 256);
                assert_source_size(&surface);

                let content = vec![0u8; 200 * 140 * 4];
                surface.upload_content(&gpu, 200, 140, &content);
                assert_eq!(
                    surface.content_tex.as_ref().map(|(cw, ch, _, _)| (*cw, *ch)),
                    Some((256, 256))
                );
                surface.present(&gpu, &constants);
            }
            DestroyWindow(hwnd);
        }
    }

    #[test]
    fn gpu_fusion_bridge_sdf_and_shadow() {
        assert_eq!(size_of::<FusionBridgeConstants>() % 16, 0);
        let _ui = crate::native_ui_test_lock();
        let Some(mut gpu) = GpuDevice::new() else {
            return;
        };

        unsafe {
            let mut wall = vec![0u8; 256 * 256 * 4];
            for y in 0..256 {
                for x in 0..256 {
                    let i = (y * 256 + x) * 4;
                    wall[i] = 40;
                    wall[i + 1] = 120;
                    wall[i + 2] = 200;
                    wall[i + 3] = 255;
                }
            }
            assert!(gpu.set_wallpaper_bgra(256, 256, &wall));

            // L-junction cluster: top-left pane [40..140, 40..120] and wider bottom pane [40..200, 120..200].
            // Concave corner forms at (140, 120), leaving a bridge fillet outside member_windows.
            let panes = [
                crate::fusion::geometry::Pane {
                    id: 1,
                    rect: [40.0, 40.0, 140.0, 120.0],
                    radius: 20.0,
                    dpi: 1.0,
                    topmost: false,
                },
                crate::fusion::geometry::Pane {
                    id: 2,
                    rect: [40.0, 120.0, 200.0, 200.0],
                    radius: 20.0,
                    dpi: 1.0,
                    topmost: false,
                },
            ];
            let (w, h) = (256, 256);
            let (rt_tex, _rt_srv, rt_rtv) =
                create_rt_texture(&gpu.device, w, h).expect("rt texture");

            let packed = crate::fusion::geometry::with_outline(&panes, |outline| {
                outline.pack_gpu_segments([0.0, 0.0])
            })
            .expect("pack_gpu_segments fits");
            let (fuse_lines, fuse_line_count, fuse_corner_c, fuse_corner_uv, fuse_corner_count) =
                packed;

            let mut constants = FusionBridgeConstants {
                bridge_rect: [0.0, 0.0, w as f32, h as f32],
                desktop_rect: [0.0, 0.0, 256.0, 256.0],
                params: [1.0, 48.0, 1.0, 0.0],
                accent_flags: [0.2, 0.4, 0.8, 1.0],
                counts: [2, 2, fuse_line_count, fuse_corner_count],
                fuse_lines,
                fuse_corner_c,
                fuse_corner_uv,
                ..Default::default()
            };
            constants.pane_rects[0] = panes[0].rect;
            constants.pane_rects[1] = panes[1].rect;
            constants.pane_meta[0] = [20.0, 1.0, 1.0, 0.0];
            constants.pane_meta[1] = [20.0, 1.0, 1.0, 0.0];
            constants.member_windows[0] = [36.0, 36.0, 144.0, 124.0];
            constants.member_windows[1] = [36.0, 116.0, 204.0, 204.0];

            // 1. Exact contour SDF mode (morph_k == 0.0)
            gpu.render_fusion_bridge(&rt_rtv, w, h, &constants, None);

            // Far outside pad_f=48 must be transparent.
            let far = read_rt_pixel(&gpu, &rt_tex, w, h, 0, 0);
            assert_eq!(far[3], 0);

            // Member window interior must be cut out cleanly.
            let inside_win = read_rt_pixel(&gpu, &rt_tex, w, h, 90, 80);
            assert_eq!(inside_win[3], 0);

            // Inner 3dp underlap band along member window concave/join seam (d < -1.0) stays opaque.
            let underlap = read_rt_pixel(&gpu, &rt_tex, w, h, 142, 118);
            assert!(
                underlap[3] >= 240,
                "expected 3dp inner underlap at join seam, got {underlap:?}"
            );

            // Concave fillet region just outside member_windows[0] right edge (x=145, y=115) must be opaque glass.
            let fillet = read_rt_pixel(&gpu, &rt_tex, w, h, 145, 115);
            assert!(fillet[3] >= 240, "expected opaque fillet, got {fillet:?}");

            // Shadow below bottom edge (x=120, y=210) must be semi-transparent black.
            let shadow = read_rt_pixel(&gpu, &rt_tex, w, h, 120, 210);
            assert!(
                shadow[3] > 5 && shadow[3] < 120,
                "expected soft shadow alpha, got {shadow:?}"
            );
            assert_eq!(&shadow[..3], &[0, 0, 0]);

            // 2. Smooth liquid morph smin mode (morph_k > 0.0)
            constants.params[3] = 32.0;
            gpu.render_fusion_bridge(&rt_rtv, w, h, &constants, None);

            let inside_win_smin = read_rt_pixel(&gpu, &rt_tex, w, h, 90, 80);
            assert_eq!(inside_win_smin[3], 0);
            let fillet_smin = read_rt_pixel(&gpu, &rt_tex, w, h, 145, 115);
            assert!(
                fillet_smin[3] >= 240,
                "expected opaque smin fillet, got {fillet_smin:?}"
            );
            let shadow_smin = read_rt_pixel(&gpu, &rt_tex, w, h, 120, 210);
            assert!(
                shadow_smin[3] > 5 && shadow_smin[3] < 120,
                "expected soft smin shadow, got {shadow_smin:?}"
            );

            // 3. Per-pane delta shifts bridge color cleanly over shared wallpaper.
            constants.pane_delta[0] = [-0.25, 0.25, 0.25, 1.0];
            constants.pane_delta[1] = [-0.25, 0.25, 0.25, 1.0];
            gpu.render_fusion_bridge(&rt_rtv, w, h, &constants, None);
            let shifted = read_rt_pixel(&gpu, &rt_tex, w, h, 145, 115);
            assert_eq!(shifted[3], fillet_smin[3]);
            assert!(
                shifted[2] + 20 < fillet_smin[2]
                    && shifted[1] > fillet_smin[1] + 20
                    && shifted[0] > fillet_smin[0] + 20,
                "expected pane_delta to shift bridge BGR from {fillet_smin:?} to {shifted:?}"
            );

            // 4. Affine material field texture (half gain + warm bias) overrides pane_delta.
            let field_rgba = [
                192u8, 128, 64, 128, 192, 128, 64, 128, 192, 128, 64, 128, 192, 128, 64, 128,
            ];
            let (_field_tex, field_srv) = create_texture2d(
                &gpu.device,
                2,
                2,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                Some(&field_rgba),
                false,
            )
            .expect("field texture");
            constants.field_uv = [2.0, 2.0, 256.0, 1.0];
            gpu.render_fusion_bridge(&rt_rtv, w, h, &constants, Some(&field_srv));
            let affine_px = read_rt_pixel(&gpu, &rt_tex, w, h, 145, 115);
            assert_eq!(affine_px[3], fillet_smin[3]);
            assert!(
                affine_px[2] > shifted[2] + 40 && affine_px[0] + 40 < shifted[0],
                "expected affine bridge_field_tex warm shift from {shifted:?} to {affine_px:?}"
            );
        }
    }
}
