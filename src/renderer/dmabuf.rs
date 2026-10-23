//! Zero-copy client buffers: DMA-BUFs imported straight into the renderer's Vulkan device.
//!
//! The wgpu device is created by hand (`create_device`) so that the Vulkan extensions needed for importing are
//! switched on. When the driver lacks them, or the backend is not Vulkan, everything falls back to the normal device
//! and clients simply keep using shared memory.

use super::{Renderer, SurfaceTex};
use ash::{ext, khr, vk};
use std::ffi::{c_char, CStr};
use std::os::fd::{AsRawFd, OwnedFd};
use wgpu::hal::api::Vulkan;

pub const FOURCC_ARGB8888: u32 = 0x3432_5241; // 'AR24'
pub const FOURCC_XRGB8888: u32 = 0x3432_5258; // 'XR24'
pub const FOURCC_ABGR8888: u32 = 0x3432_4241; // 'AB24'
pub const FOURCC_XBGR8888: u32 = 0x3432_4258; // 'XB24'

/// Formats (and their Vulkan / wgpu twins) that clients may share. Only plain 8-bit RGBA layouts for now.
fn format_of(fourcc: u32) -> Option<(vk::Format, wgpu::TextureFormat, bool)> {
    Some(match fourcc {
        FOURCC_ARGB8888 => (vk::Format::B8G8R8A8_UNORM, wgpu::TextureFormat::Bgra8Unorm, false),
        FOURCC_XRGB8888 => (vk::Format::B8G8R8A8_UNORM, wgpu::TextureFormat::Bgra8Unorm, true),
        FOURCC_ABGR8888 => (vk::Format::R8G8B8A8_UNORM, wgpu::TextureFormat::Rgba8Unorm, false),
        FOURCC_XBGR8888 => (vk::Format::R8G8B8A8_UNORM, wgpu::TextureFormat::Rgba8Unorm, true),
        _ => return None,
    })
}

/// What the GPU can import: (fourcc, modifier) pairs, and the DRM render node of the device (for clients choosing
/// which GPU to draw on).
#[derive(Clone, Debug, Default)]
pub struct DmabufCaps {
    pub formats: Vec<(u32, u64)>,
    pub render_dev: Option<u64>,
}

/// One plane of a shared buffer.
pub struct DmaPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

const WANTED: [&CStr; 4] = [khr::external_memory_fd::NAME, ext::external_memory_dma_buf::NAME, ext::image_drm_format_modifier::NAME, khr::image_format_list::NAME];
/// extensions the ones above build on; core in Vulkan 1.1 / 1.2, but harmless to enable
const SUPPORT: [&CStr; 4] = [khr::bind_memory2::NAME, khr::sampler_ycbcr_conversion::NAME, khr::external_memory::NAME, khr::dedicated_allocation::NAME];

/// A wgpu device and queue; with the DMA-BUF import extensions when the driver has them.
pub fn create_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let plain = || pollster::block_on(adapter.request_device(desc)).map_err(|e| e.to_string());
    if adapter.get_info().backend != wgpu::Backend::Vulkan || std::env::var_os("MEMRIUM_NO_DMABUF").is_some() {
        return plain();
    }
    // SAFETY: only raw Vulkan handles owned by this function are created, and they are handed to wgpu-hal
    match unsafe { vulkan_device(adapter, desc) } {
        Ok(v) => Ok(v),
        Err(e) => {
            eprintln!("dmabuf: {e}; GPU buffer sharing is off");
            plain()
        }
    }
}

unsafe fn vulkan_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let open = adapter
        .as_hal::<Vulkan, _, _>(|hal| -> Result<wgpu::hal::OpenDevice<Vulkan>, String> {
            let hal = hal.ok_or("not a Vulkan adapter")?;
            let instance = hal.shared_instance().raw_instance();
            let phys = hal.raw_physical_device();
            let supported = instance.enumerate_device_extension_properties(phys).map_err(|e| e.to_string())?;
            let has = |name: &CStr| supported.iter().any(|e| e.extension_name_as_c_str().is_ok_and(|n| n == name));
            if let Some(missing) = WANTED.iter().find(|w| !has(w)) {
                return Err(format!("the driver lacks {}", missing.to_string_lossy()));
            }
            let features = desc.required_features;
            let mut exts = hal.required_device_extensions(features);
            for e in WANTED.iter().chain(SUPPORT.iter()).chain([ext::physical_device_drm::NAME].iter()) {
                if has(e) && !exts.contains(e) {
                    exts.push(e);
                }
            }
            let mut pdf = hal.physical_device_features(&exts, features);
            let family = instance
                .get_physical_device_queue_family_properties(phys)
                .iter()
                .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .ok_or("no graphics queue")? as u32;
            let priorities = [1.0f32];
            let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
            let names: Vec<*const c_char> = exts.iter().map(|e| e.as_ptr()).collect();
            let info = vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_extension_names(&names);
            let info = pdf.add_to_device_create(info);
            let raw = instance.create_device(phys, &info, None).map_err(|e| format!("vkCreateDevice: {e}"))?;
            hal.device_from_raw(raw, None, &exts, features, &desc.memory_hints, family, 0).map_err(|e| format!("{e:?}"))
        })?;
    adapter.create_device_from_hal(open, desc).map_err(|e| e.to_string())
}

impl DmabufCaps {
    /// Ask the GPU which formats and modifiers it can import. `None` when the device was created without the
    /// extensions.
    pub(super) fn query(device: &wgpu::Device) -> Option<DmabufCaps> {
        // SAFETY: read-only Vulkan queries on handles owned by the device
        unsafe {
            device.as_hal::<Vulkan, _, _>(|hal| {
                let hal = hal?;
                if !hal.enabled_device_extensions().contains(&ext::image_drm_format_modifier::NAME) {
                    return None;
                }
                let instance = hal.shared_instance().raw_instance();
                let phys = hal.raw_physical_device();
                let mut formats = Vec::new();
                for fourcc in [FOURCC_ARGB8888, FOURCC_XRGB8888, FOURCC_ABGR8888, FOURCC_XBGR8888] {
                    let (vkfmt, _, _) = format_of(fourcc)?;
                    // how many modifiers, then which
                    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
                    let mut props = vk::FormatProperties2::default().push_next(&mut list);
                    instance.get_physical_device_format_properties2(phys, vkfmt, &mut props);
                    let mut mods = vec![vk::DrmFormatModifierPropertiesEXT::default(); list.drm_format_modifier_count as usize];
                    let mut list = vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut mods);
                    let mut props = vk::FormatProperties2::default().push_next(&mut list);
                    instance.get_physical_device_format_properties2(phys, vkfmt, &mut props);
                    for m in &mods {
                        // single-plane layouts that can be sampled (compressed layouts carry extra planes and are left out)
                        if m.drm_format_modifier_plane_count != 1 || !m.drm_format_modifier_tiling_features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE) {
                            continue;
                        }
                        let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                        let mut modifier = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default().drm_format_modifier(m.drm_format_modifier).sharing_mode(vk::SharingMode::EXCLUSIVE);
                        let query = vk::PhysicalDeviceImageFormatInfo2::default()
                            .format(vkfmt)
                            .ty(vk::ImageType::TYPE_2D)
                            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                            .usage(vk::ImageUsageFlags::SAMPLED)
                            .push_next(&mut external)
                            .push_next(&mut modifier);
                        let mut out = vk::ImageFormatProperties2::default();
                        if instance.get_physical_device_image_format_properties2(phys, &query, &mut out).is_ok() {
                            formats.push((fourcc, m.drm_format_modifier));
                        }
                    }
                }
                let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
                let mut p2 = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
                instance.get_physical_device_properties2(phys, &mut p2);
                let render_dev = (drm.has_render != 0).then(|| libc::makedev(drm.render_major as u32, drm.render_minor as u32));
                (!formats.is_empty()).then_some(DmabufCaps { formats, render_dev })
            })
        }
    }
}

impl Renderer {
    /// Formats clients may share through DMA-BUF, when the GPU can import them.
    pub fn dmabuf_caps(&self) -> Option<&DmabufCaps> {
        self.dmabuf.as_ref()
    }

    /// Import a client's DMA-BUF as a texture, without copying it.
    pub fn import_dmabuf(&self, width: u32, height: u32, fourcc: u32, modifier: u64, planes: &[DmaPlane]) -> Result<SurfaceTex, String> {
        if self.dmabuf.is_none() {
            return Err("DMA-BUF import is not available".into());
        }
        let [plane] = planes else { return Err(format!("{} planes: only single-plane buffers are supported", planes.len())) };
        let (vkfmt, wfmt, opaque) = format_of(fourcc).ok_or_else(|| format!("unsupported format {fourcc:#x}"))?;
        // SAFETY: the image and its memory are created here from the client's fd and handed to wgpu-hal, which frees
        // both through the drop callback
        unsafe {
            let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
            let hal_tex = self.device.as_hal::<Vulkan, _, _>(|hal| -> Result<wgpu::hal::vulkan::Texture, String> {
                let hal = hal.ok_or("no Vulkan device")?;
                let raw = hal.raw_device().clone();
                let instance = hal.shared_instance().raw_instance();

                let layouts = [vk::SubresourceLayout { offset: plane.offset as u64, size: 0, row_pitch: plane.stride as u64, array_pitch: 0, depth_pitch: 0 }];
                let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default().drm_format_modifier(modifier).plane_layouts(&layouts);
                let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
                let image_info = vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vkfmt)
                    .extent(vk::Extent3D { width, height, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                    .usage(vk::ImageUsageFlags::SAMPLED)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .push_next(&mut external)
                    .push_next(&mut modifier_info);
                let image = raw.create_image(&image_info, None).map_err(|e| format!("vkCreateImage: {e}"))?;
                let fail = |e: String| {
                    raw.destroy_image(image, None);
                    e
                };

                let reqs = raw.get_image_memory_requirements(image);
                let fd_props = khr::external_memory_fd::Device::new(instance, &raw);
                let mut fd_mem = vk::MemoryFdPropertiesKHR::default();
                fd_props
                    .get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT, plane.fd.as_raw_fd(), &mut fd_mem)
                    .map_err(|e| fail(format!("vkGetMemoryFdPropertiesKHR: {e}")))?;
                let bits = reqs.memory_type_bits & fd_mem.memory_type_bits;
                if bits == 0 {
                    return Err(fail("no memory type fits the buffer".into()));
                }
                let mem_props = instance.get_physical_device_memory_properties(hal.raw_physical_device());
                let type_index = (0..mem_props.memory_type_count)
                    .filter(|i| bits & (1 << i) != 0)
                    .max_by_key(|&i| mem_props.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL))
                    .unwrap_or(0);

                // Vulkan takes ownership of the fd it imports: give it its own copy
                let fd = plane.fd.try_clone().map_err(|e| fail(format!("dup: {e}")))?;
                let raw_fd = std::os::fd::IntoRawFd::into_raw_fd(fd);
                let mut import = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT).fd(raw_fd);
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
                let alloc = vk::MemoryAllocateInfo::default().allocation_size(reqs.size).memory_type_index(type_index).push_next(&mut import).push_next(&mut dedicated);
                let memory = match raw.allocate_memory(&alloc, None) {
                    Ok(m) => m,
                    Err(e) => {
                        // the import failed, so the fd is still ours to close
                        libc::close(raw_fd);
                        return Err(fail(format!("vkAllocateMemory: {e}")));
                    }
                };
                if let Err(e) = raw.bind_image_memory(image, memory, 0) {
                    raw.free_memory(memory, None);
                    return Err(fail(format!("vkBindImageMemory: {e}")));
                }
                let hal_desc = wgpu::hal::TextureDescriptor {
                    label: Some("client dmabuf"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wfmt,
                    usage: wgpu::TextureUses::RESOURCE,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                };
                let cleanup = raw.clone();
                Ok(wgpu::hal::vulkan::Device::texture_from_raw(
                    image,
                    &hal_desc,
                    Some(Box::new(move || {
                        cleanup.destroy_image(image, None);
                        cleanup.free_memory(memory, None);
                    })),
                ))
            })?;
            let tex = self.device.create_texture_from_hal::<Vulkan>(
                hal_tex,
                &wgpu::TextureDescriptor {
                    label: Some("client dmabuf"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wfmt,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            );
            Ok(SurfaceTex::from_texture(tex, (width, height), opaque))
        }
    }
}
