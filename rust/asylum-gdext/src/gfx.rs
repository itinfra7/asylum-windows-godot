//! Drawing: the whole frame is one MultiMesh (unit quad, Transform2D + colour + custom data)
//! on a child canvas item, textured with the atlas and drawn with res://shaders/sprite.gdshader.

use std::sync::Arc;
use std::time::Instant;

use asylum_core::atlas::Atlas;
use asylum_core::render::{finalize, FLOATS_PER_INSTANCE};
use asylum_core::runtime::PublishedFrame;
use godot::classes::image::Format;
use godot::classes::mesh::{ArrayType, PrimitiveType};
use godot::classes::rendering_server::{CanvasItemTextureFilter, MultimeshTransformFormat};
use godot::classes::{ArrayMesh, Image, ImageTexture, RenderingServer, Shader, ShaderMaterial};
use godot::obj::EngineEnum;
use godot::prelude::*;

const SHADER_PATH: &str = "res://shaders/sprite.gdshader";
/// Initial MultiMesh capacity (grows by doubling when a frame needs more).
const INITIAL_INSTANCES: usize = 4096;

pub struct Gfx {
    canvas_item: Rid,
    multimesh: Rid,
    // Kept alive for the RIDs used by the canvas item.
    _mesh: Gd<ArrayMesh>,
    _texture: Gd<ImageTexture>,
    _material: Gd<ShaderMaterial>,
    capacity: usize,
    /// finalize() output (reused).
    inst: Vec<f32>,
    /// Two upload buffers used alternately, so the one handed to the (threaded) renderer
    /// last frame is normally released before we write into it again (no copy-on-write).
    packed: [PackedFloat32Array; 2],
    flip: usize,
    clear: Option<u32>,
    /// Last upload (frame tick time + alpha) to skip redundant uploads.
    last: Option<(Instant, f32)>,
    /// Scale of the canvas item (1, 1 unless the latest frame was laid out for another view).
    stretch: (f32, f32),
    freed: bool,
}

fn rgba8(c: u32) -> Color {
    Color::from_rgba8((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8)
}

impl Gfx {
    /// Creates the atlas texture, material, mesh and canvas item under `parent`.
    pub fn new(parent: Rid, atlas: &Arc<Atlas>) -> Result<Gfx, String> {
        let bytes = PackedByteArray::from(atlas.rgba.as_slice());
        let image = Image::create_from_data(atlas.width as i32, atlas.height as i32, false, Format::RGBA8, &bytes)
            .ok_or("could not create the atlas image")?;
        let texture = ImageTexture::create_from_image(&image).ok_or("could not create the atlas texture")?;

        let shader = godot::tools::try_load::<Shader>(SHADER_PATH).map_err(|e| format!("{SHADER_PATH}: {e}"))?;
        let mut material = ShaderMaterial::new_gd();
        material.set_shader(&shader);

        // Unit quad 0..1 with UV 0..1 (the shader maps UV into the atlas rectangle).
        let mut mesh = ArrayMesh::new_gd();
        let quad = [Vector2::new(0.0, 0.0), Vector2::new(1.0, 0.0), Vector2::new(1.0, 1.0), Vector2::new(0.0, 1.0)];
        let verts = PackedVector2Array::from(&quad[..]);
        let uvs = PackedVector2Array::from(&quad[..]);
        let idx = PackedInt32Array::from(&[0, 1, 2, 0, 2, 3][..]);
        let mut arrays = VarArray::new();
        arrays.resize(ArrayType::MAX.ord() as usize, &Variant::nil());
        arrays.set(ArrayType::VERTEX.ord() as usize, &verts.to_variant());
        arrays.set(ArrayType::TEX_UV.ord() as usize, &uvs.to_variant());
        arrays.set(ArrayType::INDEX.ord() as usize, &idx.to_variant());
        mesh.add_surface_from_arrays(PrimitiveType::TRIANGLES, &arrays);

        let mut rs = RenderingServer::singleton();
        let multimesh = rs.multimesh_create();
        rs.multimesh_set_mesh(multimesh, mesh.get_rid());
        let canvas_item = rs.canvas_item_create();
        rs.canvas_item_set_parent(canvas_item, parent);
        rs.canvas_item_set_material(canvas_item, material.get_rid());
        // The sharp-pixel shader needs bilinear sampling.
        rs.canvas_item_set_default_texture_filter(canvas_item, CanvasItemTextureFilter::LINEAR);
        rs.canvas_item_add_multimesh_ex(canvas_item, multimesh).texture(texture.get_rid()).done();

        let mut g = Gfx {
            canvas_item,
            multimesh,
            _mesh: mesh,
            _texture: texture,
            _material: material,
            capacity: 0,
            inst: Vec::with_capacity(INITIAL_INSTANCES * FLOATS_PER_INSTANCE),
            packed: [PackedFloat32Array::new(), PackedFloat32Array::new()],
            flip: 0,
            clear: None,
            last: None,
            stretch: (1.0, 1.0),
            freed: false,
        };
        g.allocate(INITIAL_INSTANCES);
        Ok(g)
    }

    fn allocate(&mut self, instances: usize) {
        let mut rs = RenderingServer::singleton();
        rs.multimesh_allocate_data_ex(self.multimesh, instances as i32, MultimeshTransformFormat::TRANSFORM_2D)
            .color_format(true)
            .custom_data_format(true)
            .done();
        // Positions are already in view space; skip AABB computation and culling.
        rs.multimesh_set_custom_aabb(
            self.multimesh,
            Aabb::new(Vector3::new(-1.0e6, -1.0e6, -1.0), Vector3::new(2.0e6, 2.0e6, 2.0)),
        );
        rs.multimesh_set_visible_instances(self.multimesh, 0);
        for p in &mut self.packed {
            p.resize(instances * FLOATS_PER_INSTANCE);
        }
        self.capacity = instances;
        self.last = None;
    }

    /// Interpolates, packs and uploads `frame` (one buffer upload + visible count). `view` is
    /// the current logical view: a frame laid out for another view (the tick after a
    /// resolution or mode change) is scaled to it, so it still fills the screen.
    pub fn draw(&mut self, pf: &PublishedFrame, alpha: f32, view: (f32, f32)) {
        let frame = &*pf.frame;
        let mut rs = RenderingServer::singleton();
        if self.clear != Some(frame.clear) {
            self.clear = Some(frame.clear);
            rs.set_default_clear_color(rgba8(frame.clear));
        }
        let stretch = match frame.clips.first() {
            Some(&[_, _, w, h]) if w > 0.0 && h > 0.0 && ((w - view.0).abs() > 0.5 || (h - view.1).abs() > 0.5) => (view.0 / w, view.1 / h),
            _ => (1.0, 1.0),
        };
        if stretch != self.stretch {
            self.stretch = stretch;
            let t = Transform2D::from_cols(Vector2::new(stretch.0, 0.0), Vector2::new(0.0, stretch.1), Vector2::ZERO);
            rs.canvas_item_set_transform(self.canvas_item, t);
        }
        let key = (pf.tick_time, alpha);
        if self.last == Some(key) {
            return;
        }
        self.last = Some(key);
        let n = finalize(frame, alpha, &mut self.inst);
        if n > self.capacity {
            self.allocate(n.next_power_of_two());
            self.last = Some(key);
        }
        if n > 0 {
            let len = n * FLOATS_PER_INSTANCE;
            let pk = &mut self.packed[self.flip];
            self.flip ^= 1;
            pk.as_mut_slice()[..len].copy_from_slice(&self.inst[..len]);
            rs.multimesh_set_buffer(self.multimesh, pk);
        }
        rs.multimesh_set_visible_instances(self.multimesh, n as i32);
    }

    /// Releases the rendering-server objects (call before the node leaves the tree).
    pub fn free(&mut self) {
        if !self.freed {
            self.freed = true;
            let mut rs = RenderingServer::singleton();
            rs.free_rid(self.canvas_item);
            rs.free_rid(self.multimesh);
        }
    }
}
