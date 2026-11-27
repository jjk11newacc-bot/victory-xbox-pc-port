//! Displays ambient-lit geometry when the title's final SDR image is dark.

use super::*;
use crate::resource::HostTexture;
use std::sync::Arc;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("VICTORY_SCENE_FALLBACK").is_some_and(|v| v == "1")
            && std::env::var_os("VICTORY_RAY_TRACING").is_some_and(|v| v == "0")
    })
}

pub(super) struct Scene {
    frame: u64,
    albedo: Arc<HostTexture>,
    depth: Arc<HostTexture>,
}

static SCENE: Mutex<Option<Scene>> = Mutex::new(None);

/// The deferred material pass binds six targets, beginning with typeless
/// RGBA8 albedo and ending with a second RGBA8 material target.
pub(crate) fn note(targets: &[usize], depth: usize) {
    let frame = crate::queue::presented() + 1;
    let mut slot = SCENE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|scene| scene.frame >= frame) {
        return;
    }
    let get = |address| crate::resource::existing_host_copy(address as crate::object::This);
    let (Some(albedo), Some(depth)) = (get(targets[0]), get(depth)) else {
        return;
    };
    // SAFETY: the Arc keeps the underlying textures alive.
    let (a, d) = unsafe { (albedo.texture.GetDesc(), depth.texture.GetDesc()) };
    if a.Format.0 != 27 || !matches!(d.Format.0, 19 | 44)
        || a.Width < 1280 || a.Height < 720 || (a.Width, a.Height) != (d.Width, d.Height)
        || [a, d].iter().any(|r| {
            r.SampleDesc.Count != 1 || r.DepthOrArraySize != 1 || r.MipLevels != 1
                || r.Flags.0 & D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE.0 != 0
                || r.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
        })
    {
        return;
    }
    *slot = Some(Scene { frame, albedo, depth });
}

pub(super) fn take(frame: u64) -> Option<Scene> {
    if !enabled() {
        return None;
    }
    let mut slot = SCENE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|scene| scene.frame <= frame) {
        return slot.take().filter(|scene| scene.frame == frame);
    }
    None
}

const PS: &str = r#"
#define RS "DescriptorTable(SRV(t0, numDescriptors=3), visibility=SHADER_VISIBILITY_PIXEL)"
Texture2D<float4> finalImage : register(t0);
Texture2D<float4> albedo : register(t1);
Texture2D<float> depth : register(t2);

[RootSignature(RS)]
float4 composite(float4 p : SV_Position) : SV_Target {
    int2 at = int2(p.xy);
    float4 color = finalImage.Load(int3(at, 0));
    float bright = max(color.r, max(color.g, color.b));
    float dark = min(color.r, min(color.g, color.b));
    if (bright > 0.25 || bright - dark > 0.07)
        return color;
    uint w, h, sw, sh;
    finalImage.GetDimensions(w, h);
    depth.GetDimensions(sw, sh);
    int2 texel = min(int2(p.xy * (float2(sw, sh) / float2(w, h))), int2(sw - 1, sh - 1));
    float z = depth.Load(int3(texel, 0));
    float3 material = albedo.Load(int3(texel, 0)).rgb;
    bool hasDepth = z > 0.0 && z < 1.0;
    if (!hasDepth && max(material.r, max(material.g, material.b)) < 0.01)
        return color;
    float proximity = hasDepth ? saturate(sqrt(sqrt(z)) * 2.0) : 0.0;
    float level = 0.60 + 0.28 * proximity;
    float floor = hasDepth ? 0.25 + 0.30 * proximity : 0.0;
    float3 diffuse = floor.xxx + sqrt(saturate(material)) * max(0.0, level - floor);
    return float4(max(color.rgb, diffuse), color.a);
}
"#;

struct Pass {
    heap: ID3D12DescriptorHeap,
    root: ID3D12RootSignature,
    pipeline: ID3D12PipelineState,
}

fn make(device: &ID3D12Device) -> windows::core::Result<Pass> {
    use windows::core::s;
    let vs = compile(PLANE_CORNERS, s!("corner"), s!("vs_5_1"))?;
    let ps = compile(PS, s!("composite"), s!("ps_5_1"))?;
    let code = |bytes: &[u8]| D3D12_SHADER_BYTECODE {
        pShaderBytecode: bytes.as_ptr().cast(),
        BytecodeLength: bytes.len(),
    };
    // SAFETY: descriptors refer to live data for the duration of creation.
    unsafe {
        let heap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: 3,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &ps)?;
        let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            VS: code(&vs),
            PS: code(&ps),
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                DepthClipEnable: true.into(),
                ..Default::default()
            },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };
        desc.BlendState.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
        desc.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
        let pipeline = device.CreateGraphicsPipelineState(&desc)?;
        Ok(Pass { heap, root, pipeline })
    }
}

/// Records on the synchronous present list, so its three descriptors can
/// be reused once the frame completes.
pub(super) fn record(
    host: &Host, list: &ID3D12GraphicsCommandList, output: &ID3D12Resource,
    output_state: D3D12_RESOURCE_STATES, back: &ID3D12Resource, scene: &Scene,
    width: u32, height: u32,
) -> bool {
    // SAFETY: these resources are held by the frame and by the presenter.
    let (out, d) = unsafe { (output.GetDesc(), scene.depth.texture.GetDesc()) };
    if !matches!(out.Format.0, 27 | 28)
        || out.Flags.0 & D3D12_RESOURCE_FLAG_DENY_SHADER_RESOURCE.0 != 0
        || output == &scene.albedo.texture
    {
        return false;
    }
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    let Some(pass) = PASS.get_or_init(|| {
        make(&host.device).inspect_err(|error| {
            tracing::warn!(%error, "could not create the no-RT scene fallback");
        }).ok()
    }).as_ref() else { return false };

    let src_state = |resource: &HostTexture| {
        D3D12_RESOURCE_STATES(legal_state(&resource.texture, resource.state.load(Ordering::Relaxed)) as i32)
    };
    let inputs = [
        (output, output_state, 28),
        (&scene.albedo.texture, src_state(&scene.albedo), 28),
        (&scene.depth.texture, src_state(&scene.depth), if d.Format.0 == 19 { 21 } else { 46 }),
    ];
    let read = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
    let present = D3D12_RESOURCE_STATE_PRESENT;
    let render = D3D12_RESOURCE_STATE_RENDER_TARGET;
    // SAFETY: all resources outlive the synchronous list; its transitions
    // restore the title's state before returning.
    unsafe {
        let mut descriptor = pass.heap.GetCPUDescriptorHandleForHeapStart();
        let step = host.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
        for (resource, before, format) in inputs {
            let view = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT(format),
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() },
                },
            };
            host.device.CreateShaderResourceView(resource, Some(&view), descriptor);
            descriptor.ptr += step as usize;
            if before != read {
                list.ResourceBarrier(&[transition(resource, before, read)]);
            }
        }
        list.ResourceBarrier(&[transition(back, present, render)]);
        let target = host.rtv(Some(back), None);
        list.SetDescriptorHeaps(&[Some(pass.heap.clone())]);
        list.SetGraphicsRootSignature(&pass.root);
        list.SetPipelineState(&pass.pipeline);
        list.SetGraphicsRootDescriptorTable(0, pass.heap.GetGPUDescriptorHandleForHeapStart());
        list.OMSetRenderTargets(1, Some(&target), false, None);
        list.RSSetViewports(&[D3D12_VIEWPORT {
            Width: width as f32, Height: height as f32, MaxDepth: 1.0, ..Default::default()
        }]);
        list.RSSetScissorRects(&[RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 }]);
        list.IASetPrimitiveTopology(windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        list.DrawInstanced(3, 1, 0, 0);
        list.ResourceBarrier(&[transition(back, render, present)]);
        for (resource, before, _) in inputs {
            if before != read {
                list.ResourceBarrier(&[transition(resource, read, before)]);
            }
        }
    }
    static LOGGED: OnceLock<()> = OnceLock::new();
    LOGGED.get_or_init(|| tracing::info!("presenting with the no-RT scene fallback"));
    true
}
