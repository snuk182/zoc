use gfx;
use gfx::traits::{FactoryExt};
use gfx_gl;
use fs;
use context::{Context};
use texture::{Texture, load_texture};
use pipeline::{Vertex};

#[derive(PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct MeshId{pub id: i32}

// TODO: TODO: make fields private
pub struct Mesh {
    pub slice: gfx::Slice<gfx_gl::Resources>,
    pub vertex_buffer: gfx::handle::Buffer<gfx_gl::Resources, Vertex>,
    pub texture: Texture,
    is_wire: bool,
}

impl Mesh {
    pub fn new(context: &mut Context, vertices: &[Vertex], indices: &[u16], tex: Texture) -> Mesh {
        let (v, s) = context.factory.create_vertex_buffer_with_slice(vertices, indices);
        Mesh {
            slice: s,
            vertex_buffer: v,
            texture: tex,
            is_wire: false,
        }
    }

    pub fn new_wireframe(context: &mut Context, vertices: &[Vertex], indices: &[u16]) -> Mesh {
        let (v, s) = context.factory.create_vertex_buffer_with_slice(vertices, indices);
        let texture_data = fs::load("white.png").into_inner();
        let texture = load_texture(context, &texture_data);
        Mesh {
            slice: s,
            vertex_buffer: v,
            texture: texture,
            is_wire: true,
        }
    }

    pub fn is_wire(&self) -> bool {
        self.is_wire
    }
}
