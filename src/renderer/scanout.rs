//! Zero-copy output: images the renderer's Vulkan device draws into and the display controller scans out directly.
//!
//! The image is allocated by Vulkan with an exportable DMA-BUF; the session turns the fd into a DRM framebuffer. Only
//! meaningful when the display is driven by the same GPU as the renderer (`primary_dev`); otherwise the session keeps
//! its dumb buffers.

use ash::{ext, khr, vk};
use std::os::fd::{FromRawFd, OwnedFd};
use wgpu::hal::api::Vulkan;

const VKFMT: vk::Format = vk::Format::B8G8R8A8_UNORM;
const USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw() | vk::ImageUsageFlags::TRANSFER_DST.as_raw());
const LINEAR: u64 = 0;

/// An image that can be put on a screen.
pub struct ScanoutImage {
    pub texture: wgpu::Texture,
    pub fd: OwnedFd,
    pub modifier: u64,
    pub pitch: u32,
    pub offset: u32,
}

/// The device number of the DRM primary node (`/dev/dri/cardN`) of the GPU that renders; compare it with the `st_rdev`
/// of the card a monitor hangs on.
pub fn primary_dev(device: &wgpu::Device) -> Option<u64> {
    // SAFETY: read-only Vulkan query
    unsafe {
        device.as_hal::<Vulkan, _, _>(|hal| {
            let hal = hal?;
            if !hal.enabled_device_extensions().contains(&ext::physical_device_drm::NAME) {
                return None;
            }
            let instance = hal.shared_instance().raw_instance();
            let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
            let mut p2 = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
            instance.get_physical_device_properties2(hal.raw_physical_device(), &mut p2);
            (drm.has_primary != 0).then(|| libc::makedev(drm.primary_major as u32, drm.primary_minor as u32))
        })
    }
}

/// Layouts the GPU can draw into and export as one plane, best first (linear last: it is the slowest to draw).
pub fn scanout_modifiers(device: &wgpu::Device) -> Vec<u64> {
    // SAFETY: read-only Vulkan queries
    unsafe {
        device.as_hal::<Vulkan, _, _>(|hal| {
                let Some(hal) = hal else { return Vec::new() };
                for need in [khr::external_memory_fd::NAME, ext::external_memory_dma_buf::NAME, ext::image_drm_format_modifier::NAME] {
                    if !hal.enabled_device_extensions().contains(&need) {
                        return Vec::new();
                    }
                }
                let instance = hal.shared_instance().raw_instance();
                let phys = hal.raw_physical_device();
                let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
                let mut props = vk::FormatProperties2::default().push_next(&mut list);
                instance.get_physical_device_format_properties2(phys, VKFMT, &mut props);
                let mut mods = vec![vk::DrmFormatModifierPropertiesEXT::default(); list.drm_format_modifier_count as usize];
                let mut list = vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut mods);
                let mut props = vk::FormatProperties2::default().push_next(&mut list);
                instance.get_physical_device_format_properties2(phys, VKFMT, &mut props);
                let want = vk::FormatFeatureFlags::COLOR_ATTACHMENT | vk::FormatFeatureFlags::TRANSFER_DST;
                let mut out: Vec<u64> = Vec::new();
                for m in &mods {
                    if m.drm_format_modifier_plane_count != 1 || !m.drm_format_modifier_tiling_features.contains(want) {
                        continue;
                    }
                    let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                    let mut modifier = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default().drm_format_modifier(m.drm_format_modifier).sharing_mode(vk::SharingMode::EXCLUSIVE);
                    let query = vk::PhysicalDeviceImageFormatInfo2::default().format(VKFMT).ty(vk::ImageType::TYPE_2D).tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT).usage(USAGE).push_next(&mut external).push_next(&mut modifier);
                    let mut ext_props = vk::ExternalImageFormatProperties::default();
                    let mut res = vk::ImageFormatProperties2::default().push_next(&mut ext_props);
                    if instance.get_physical_device_image_format_properties2(phys, &query, &mut res).is_ok() && ext_props.external_memory_properties.external_memory_features.contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE) {
                        out.push(m.drm_format_modifier);
                    }
                }
                // X-tiled (0x...01) draws slower than the newer layouts
                out.sort_by_key(|&m| (m == LINEAR, m == 0x0100_0000_0000_0001));
                out
            })
    }
}

/// Allocate a `w`x`h` image with exactly this modifier.
pub fn create_scanout(device: &wgpu::Device, w: u32, h: u32, modifier: u64) -> Result<ScanoutImage, String> {
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    // SAFETY: the image and its memory are created here and handed to wgpu-hal, which frees both through the drop callback
    unsafe {
        let made = device.as_hal::<Vulkan, _, _>(|hal| -> Result<(wgpu::hal::vulkan::Texture, OwnedFd, u64, u32, u32), String> {
            let hal = hal.ok_or("no Vulkan device")?;
            let raw = hal.raw_device().clone();
            let instance = hal.shared_instance().raw_instance();

            let mods = [modifier];
            let mut list = vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&mods);
            let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(VKFMT)
                .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(USAGE)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut external)
                .push_next(&mut list);
            let image = raw.create_image(&info, None).map_err(|e| format!("vkCreateImage: {e}"))?;
            let fail = |e: String| {
                raw.destroy_image(image, None);
                e
            };
            let reqs = raw.get_image_memory_requirements(image);
            let mem_props = instance.get_physical_device_memory_properties(hal.raw_physical_device());
            let type_index = (0..mem_props.memory_type_count)
                .filter(|i| reqs.memory_type_bits & (1 << i) != 0)
                .max_by_key(|&i| mem_props.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
                .ok_or_else(|| fail("no memory type".into()))?;
            let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let alloc = vk::MemoryAllocateInfo::default().allocation_size(reqs.size).memory_type_index(type_index).push_next(&mut export).push_next(&mut dedicated);
            let memory = raw.allocate_memory(&alloc, None).map_err(|e| fail(format!("vkAllocateMemory: {e}")))?;
            let fail2 = |e: String| {
                raw.free_memory(memory, None);
                fail(e)
            };
            raw.bind_image_memory(image, memory, 0).map_err(|e| fail2(format!("vkBindImageMemory: {e}")))?;

            let mut mp = vk::ImageDrmFormatModifierPropertiesEXT::default();
            ext::image_drm_format_modifier::Device::new(instance, &raw).get_image_drm_format_modifier_properties(image, &mut mp).map_err(|e| fail2(format!("modifier: {e}")))?;
            let layout = raw.get_image_subresource_layout(image, vk::ImageSubresource { aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT, mip_level: 0, array_layer: 0 });
            let get = vk::MemoryGetFdInfoKHR::default().memory(memory).handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let raw_fd = khr::external_memory_fd::Device::new(instance, &raw).get_memory_fd(&get).map_err(|e| fail2(format!("vkGetMemoryFdKHR: {e}")))?;
            let fd = OwnedFd::from_raw_fd(raw_fd);

            let hal_desc = wgpu::hal::TextureDescriptor {
                label: Some("scanout"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUses::COPY_DST | wgpu::TextureUses::COLOR_TARGET,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            };
            let cleanup = raw.clone();
            let tex = wgpu::hal::vulkan::Device::texture_from_raw(
                image,
                &hal_desc,
                Some(Box::new(move || {
                    cleanup.destroy_image(image, None);
                    cleanup.free_memory(memory, None);
                })),
            );
            Ok((tex, fd, mp.drm_format_modifier, layout.row_pitch as u32, layout.offset as u32))
        })?;
        let (hal_tex, fd, modifier, pitch, offset) = made;
        let texture = device.create_texture_from_hal::<Vulkan>(
            hal_tex,
            &wgpu::TextureDescriptor {
                label: Some("scanout"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        );
        Ok(ScanoutImage { texture, fd, modifier, pitch, offset })
    }
}

impl ScanoutImage {
    /// Paint the image black (new memory holds whatever was there before).
    pub fn clear(&self, enc: &mut wgpu::CommandEncoder) {
        let view = self.texture.create_view(&Default::default());
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scanout clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
}
