//! SDR display profiles for the supported Series X retail image.
use anyhow::{Context, Result, ensure};
use victory_formats::texture::{Dimension, Swizzle, Texture, TextureDesc};
use victory_loader::PreparedImage;

use super::fsr_preset::FsrPreset;

const RETAIL_BUILD: &str = "b99f876b74d4a77f03af5f878223b27bd647b5c35a79d5decd9f716504239e93";
const SDR_MODE: u32 = 0x0615_c100;
const STOCK_SDR_MODE: [u8; 16] = [0, 15, 0, 0, 112, 8, 0, 0, 60, 0, 0, 0, 1, 0, 0, 0];

pub(super) fn prepare(image: &mut PreparedImage, render_1080p: bool, output_1080p: bool) -> Result<()> {
    let (width, height) = if output_1080p { (1920u32, 1080u32) } else { (2560u32, 1440u32) };
    ensure!(
        image.original_hash.to_hex().as_str() == RETAIL_BUILD,
        "Native display profile is unavailable for this retail game build"
    );

    let mode = field::<16>(image, SDR_MODE)?;
    ensure!(
        *mode == STOCK_SDR_MODE,
        "Unexpected Series X SDR mode"
    );
    mode[..4].copy_from_slice(&width.to_le_bytes());
    mode[4..8].copy_from_slice(&height.to_le_bytes());

    // Two 64 KiB-aligned, tiled RGBA8 scanout buffers. The reservation is
    // rounded to 2 MiB, matching the title's allocation check.
    let texture = Texture::new(&TextureDesc {
        dimension: Dimension::Tex2d,
        width,
        height,
        depth_or_array: 1,
        mips: 1,
        format: 28,
        samples: 1,
        swizzle: Swizzle::SW_64KB_D_X,
    })
    .map_err(|error| anyhow::anyhow!("Native SDR layout: {error:?}"))?;
    let pool_size =
        u32::try_from((texture.size.next_multiple_of(64 * 1024) * 2).next_multiple_of(2 << 20))?;
    let reservation = field::<5>(image, 0x0247_ad4a)?;
    ensure!(
        *reservation == [0xb9, 0, 0, 0, 4],
        "Unexpected Series X scanout reservation"
    );
    reservation[1..].copy_from_slice(&pool_size.to_le_bytes());

    let scale = field::<4>(image, 0x028f_1e7e)?;
    ensure!(
        *scale == (2.0f32 / 3.0).to_le_bytes(),
        "Unexpected Series X temporal scene scale"
    );
    let (render_width, render_height) = if render_1080p {
        (1920, 1080)
    } else {
        (width, height)
    };
    *scale = (render_width as f32 / width as f32).to_le_bytes();

    // Settings uses a separate viewport setter to lay out its preview
    // panels. It loads the legacy UI dimensions, which stay at 3840×2160
    // even when the game has allocated a native-resolution UI surface. Read the
    // selected SDR mode for that viewport, before calculating its centre.
    const UI_VIEWPORT_LOAD: u32 = 0x0580_48d4;
    let viewport = field::<8>(image, UI_VIEWPORT_LOAD)?;
    ensure!(
        *viewport == [0xc5, 0xfb, 0x10, 0x0d, 0x10, 0xf0, 0x13, 0x05],
        "Unexpected Settings viewport size load"
    );
    let displacement = (SDR_MODE as i32) - (UI_VIEWPORT_LOAD as i32 + 8);
    viewport[4..].copy_from_slice(&displacement.to_le_bytes());
    tracing::info!(
        output_width = width,
        output_height = height,
        expected_render_width = render_width,
        expected_render_height = render_height,
        pool_size,
        "Series X SDR mode with built-in temporal scaling"
    );
    Ok(())
}

pub(super) fn prepare_fsr_4k(image: &mut PreparedImage, preset: FsrPreset) -> Result<()> {
    ensure!(
        image.original_hash.to_hex().as_str() == RETAIL_BUILD,
        "FSR at 4K is unavailable for this retail game build"
    );
    ensure!(
        *field::<16>(image, SDR_MODE)? == STOCK_SDR_MODE,
        "Unexpected Series X SDR mode"
    );
    ensure!(
        *field::<4>(image, 0x0615_d95c)? == (2.0f32 / 3.0).to_le_bytes(),
        "Unexpected Series X FSR scene scale"
    );
    ensure!(
        *field::<8>(image, 0x007e_9421)? == [0xb8, 6, 0, 0, 0, 0x0f, 0x45, 0xc1],
        "Unexpected Series X temporal renderer selection"
    );
    ensure!(
        *field::<14>(image, 0x028e_f69a)? == [0x0f, 0xb6, 0xc0, 0x41, 0x89, 0x86, 0x90, 0, 0, 0, 0x84, 0xc0, 0x75, 0x0a],
        "Unexpected Series X FSR render size selection"
    );
    const NATIVE_LOAD: u32 = 0x028e_f6a8;
    const QUALITY_LOAD: u32 = 0x028e_f6b2;
    if preset != FsrPreset::Quality {
        ensure!(
            *field::<10>(image, NATIVE_LOAD)? == [0xc5, 0xfa, 0x10, 0x05, 0x28, 0x78, 0x6d, 0x03, 0xeb, 0x08]
                && *field::<8>(image, QUALITY_LOAD)? == [0xc5, 0xfa, 0x10, 0x05, 0xa2, 0xe2, 0x86, 0x03],
            "Unexpected Series X FSR scale instructions"
        );
    }
    // The original selector has already run its capability check. An
    // explicit FSR session uses renderer 7 every time it queries this.
    *field::<8>(image, 0x007e_9421)? = [0xb8, 7, 0, 0, 0, 0x90, 0x90, 0x90];
    // Record the SDK mode and jump to the scaled-scene branch.
    *field::<14>(image, 0x028e_f69a)? =
        [0xb8, preset.mode(), 0, 0, 0, 0x41, 0x89, 0x86, 0x90, 0, 0, 0, 0xeb, 0x0a];
    if preset != FsrPreset::Quality {
        // The jump makes the native-scale instruction unreachable. Reuse
        // its RIP operand bytes for a private literal, leaving the constants
        // used by other passes unchanged.
        let local_scale = NATIVE_LOAD + 4;
        *field::<4>(image, local_scale)? = preset.render_scale().to_le_bytes();
        let displacement = local_scale as i32 - (QUALITY_LOAD as i32 + 8);
        *field::<4>(image, QUALITY_LOAD + 4)? = displacement.to_le_bytes();
    }
    tracing::info!(
        renderer = 7,
        preset = preset.name(),
        quality = preset.mode(),
        expected_render = ?preset.render_size(),
        output = ?[3840, 2160],
        "Selected Series X FSR with the stock 4K frontend"
    );
    Ok(())
}

fn field<const N: usize>(image: &mut PreparedImage, rva: u32) -> Result<&mut [u8; N]> {
    let offset = image
        .info
        .rva_to_file_offset(rva)
        .context("Display profile field missing")? as usize;
    image
        .bytes
        .get_mut(offset..offset + N)
        .context("Incomplete display profile field")?
        .try_into()
        .context("Invalid display profile field length")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_display_image() -> PreparedImage {
        let entries = [SDR_MODE, 0x0247_ad4a, 0x028f_1e7e, 0x0580_48d4];
        let info = victory_pe::ImageInfo {
            machine: 0x8664, characteristics: 0, timestamp: 0, subsystem: 0,
            dll_characteristics: 0, image_base: 0, size_of_image: 0x0700_0000,
            entry_rva: 0, stack_reserve: 0, stack_commit: 0,
            sections: entries.into_iter().enumerate().map(|(i, rva)| victory_pe::SectionInfo {
                name: format!("field{i}"), rva, virtual_size: 32, file_offset: (i * 32) as u32,
                file_size: 32, characteristics: 0,
            }).collect(),
            imports: vec![], delay_imports: vec![], exports: vec![], tls: None,
            has_relocations: false, has_exception_table: false, has_load_config: false, codeview: None,
        };
        let mut image = PreparedImage { info, bytes: vec![0; 128], original_hash: RETAIL_BUILD.parse().unwrap() };
        *field::<16>(&mut image, SDR_MODE).unwrap() = STOCK_SDR_MODE;
        *field::<5>(&mut image, 0x0247_ad4a).unwrap() = [0xb9, 0, 0, 0, 4];
        *field::<4>(&mut image, 0x028f_1e7e).unwrap() = (2.0f32 / 3.0).to_le_bytes();
        *field::<8>(&mut image, 0x0580_48d4).unwrap() = [0xc5, 0xfb, 0x10, 0x0d, 0x10, 0xf0, 0x13, 0x05];
        image
    }

    #[test]
    fn native_display_dimensions_scale_scanout_and_ui_are_consistent() {
        for (render1080, output1080, width, height, scale) in [
            (false, true, 1920u32, 1080u32, 1.0f32),
            (false, false, 2560, 1440, 1.0),
            (true, false, 2560, 1440, 0.75),
        ] {
            let mut image = synthetic_display_image();
            prepare(&mut image, render1080, output1080).unwrap();
            assert_eq!(*field::<4>(&mut image, SDR_MODE).unwrap(), width.to_le_bytes());
            assert_eq!(*field::<4>(&mut image, SDR_MODE + 4).unwrap(), height.to_le_bytes());
            assert_eq!(*field::<4>(&mut image, 0x028f_1e7e).unwrap(), scale.to_le_bytes());
            let texture = Texture::new(&TextureDesc { dimension: Dimension::Tex2d, width, height,
                depth_or_array: 1, mips: 1, format: 28, samples: 1, swizzle: Swizzle::SW_64KB_D_X }).unwrap();
            let bytes = (texture.size.next_multiple_of(64 * 1024) * 2).next_multiple_of(2 << 20) as u32;
            assert_eq!(*field::<4>(&mut image, 0x0247_ad4b).unwrap(), bytes.to_le_bytes());
            let disp = i32::from_le_bytes(*field::<4>(&mut image, 0x0580_48d8).unwrap());
            assert_eq!(0x0580_48dc_u32.wrapping_add_signed(disp), SDR_MODE);
        }
    }

    #[test]
    fn native_display_rejects_wrong_build_and_unexpected_fields() {
        let mut image = synthetic_display_image();
        image.original_hash = blake3::hash(b"not the supported image");
        assert!(prepare(&mut image, false, true).is_err());
        for address in [SDR_MODE, 0x0247_ad4a, 0x028f_1e7e, 0x0580_48d4] {
            let mut image = synthetic_display_image();
            field::<1>(&mut image, address).unwrap()[0] ^= 1;
            assert!(prepare(&mut image, false, true).is_err());
        }
    }

    #[test]
    fn fsr_presets_leave_the_stock_scanout_and_settings_viewport() {
        let Some(path) = std::env::var_os("VICTORY_DISPLAY_PROFILE_TEST_IMAGE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        for (preset, size) in [
            (FsrPreset::Quality, [2560, 1440]),
            (FsrPreset::Balanced, [2258, 1270]),
            (FsrPreset::Performance, [1920, 1080]),
        ] {
            let mut image = victory_loader::prepare(&bytes).unwrap();
            let scanout = *field::<5>(&mut image, 0x0247_ad4a).unwrap();
            let settings = *field::<8>(&mut image, 0x0580_48d4).unwrap();
            prepare_fsr_4k(&mut image, preset).unwrap();
            assert_eq!(*field::<16>(&mut image, SDR_MODE).unwrap(), STOCK_SDR_MODE);
            assert_eq!(*field::<5>(&mut image, 0x0247_ad4a).unwrap(), scanout);
            assert_eq!(*field::<8>(&mut image, 0x0580_48d4).unwrap(), settings);
            assert_eq!(*field::<4>(&mut image, 0x007e_9422).unwrap(), 7u32.to_le_bytes());
            assert_eq!(*field::<4>(&mut image, 0x028e_f69b).unwrap(), u32::from(preset.mode()).to_le_bytes());
            assert_eq!(preset.render_size(), size);
            let disp = i32::from_le_bytes(*field::<4>(&mut image, 0x028e_f6b6).unwrap());
            let reference = 0x028e_f6ba_u32.wrapping_add_signed(disp);
            let scale = f32::from_le_bytes(*field::<4>(&mut image, reference).unwrap());
            assert_eq!([3840.0, 2160.0].map(|w| (w * scale) as u32), size);
        }
    }
}
