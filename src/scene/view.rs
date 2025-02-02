/*
ENSnano, a 3d graphical application for DNA nanostructures.
    Copyright (C) 2021  Nicolas Levy <nicolaspierrelevy@gmail.com> and Nicolas Schabanel <nicolas.schabanel@ens-lyon.fr>

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/
//! The view module handles the drawing of the scene on texture. The scene can be drawn on the next
//! frame to be displayed, or on a "fake texture" that is used to map pixels to objects.

use super::{camera, ActionMode};
use crate::consts::*;
use crate::design::Axis;
use crate::utils::{bindgroup_manager, texture};
use crate::{DrawArea, PhySize};
use camera::{Camera, CameraPtr, Projection, ProjectionPtr};
use iced_wgpu::wgpu;
use std::cell::RefCell;
use std::rc::Rc;
use texture::Texture;
use ultraviolet::{Mat4, Rotor3, Vec3};
use wgpu::{Device, Queue};

/// A `Uniform` is a structure that manages view and projection matrices.
mod uniforms;
pub use uniforms::FogParameters;
use uniforms::Uniforms;
mod direction_cube;
mod dna_obj;
/// This modules defines a trait for drawing widget made of several meshes.
mod drawable;
mod grid;
mod grid_disc;
/// A HandleDrawer draws the widget for translating objects
mod handle_drawer;
mod instances_drawer;
mod letter;
/// A RotationWidget draws the widget for rotating objects
mod rotation_widget;

use super::maths_3d;
use crate::text::Letter;
use bindgroup_manager::{DynamicBindGroup, UniformBindGroup};
use direction_cube::*;
pub use dna_obj::{ConeInstance, DnaObject, RawDnaInstance, SphereInstance, TubeInstance};
use drawable::{Drawable, Drawer, Vertex};
pub use grid::{GridInstance, GridIntersection, GridTypeDescr};
use grid::{GridManager, GridTextures};
pub use grid_disc::GridDisc;
use handle_drawer::HandlesDrawer;
pub use handle_drawer::{HandleDir, HandleOrientation, HandlesDescriptor};
pub use instances_drawer::Instanciable;
use instances_drawer::{InstanceDrawer, RawDrawer};
pub use letter::LetterInstance;
use maths_3d::unproject_point_on_line;
use rotation_widget::RotationWidget;
pub use rotation_widget::{RotationMode, RotationWidgetDescriptor, RotationWidgetOrientation};
//use plane_drawer::PlaneDrawer;
//pub use plane_drawer::Plane;

static MODEL_BG_ENTRY: &'static [wgpu::BindGroupLayoutEntry] = &[wgpu::BindGroupLayoutEntry {
    binding: 0,
    visibility: wgpu::ShaderStage::from_bits_truncate(wgpu::ShaderStage::VERTEX.bits()),
    ty: wgpu::BindingType::Buffer {
        has_dynamic_offset: false,
        min_binding_size: None,
        ty: wgpu::BufferBindingType::Storage { read_only: true },
    },
    count: None,
}];

use crate::mediator::{Background3D, RenderingMode};

/// An object that handles the communication with the GPU to draw the scene.
pub struct View {
    /// The camera, that is in charge of producing the view and projection matrices.
    camera: CameraPtr,
    projection: ProjectionPtr,
    /// The depth texture is updated every time the size of the drawing area is modified
    depth_texture: Texture,
    /// The fake depth texture is updated every time the size of the drawing area is modified and
    /// has a sample count of 1
    fake_depth_texture: Texture,
    /// The handle drawers draw handles to translate the elements
    handle_drawers: HandlesDrawer,
    /// The rotation widget draw the widget to rotate the elements
    rotation_widget: RotationWidget,
    /// A possible update of the size of the drawing area, must be taken into account before
    /// drawing the next frame
    new_size: Option<PhySize>,
    /// The pipilines that draw the basis symbols
    letter_drawer: Vec<InstanceDrawer<LetterInstance>>,
    helix_letter_drawer: Vec<InstanceDrawer<LetterInstance>>,
    device: Rc<Device>,
    /// A bind group associated to the uniform buffer containing the view and projection matrices.
    //TODO this is currently only passed to the widgets, it could be passed to the mesh pipeline as
    //well.
    viewer: UniformBindGroup,
    models: DynamicBindGroup,
    redraw_twice: bool,
    need_redraw: bool,
    need_redraw_fake: bool,
    draw_letter: bool,
    msaa_texture: Option<wgpu::TextureView>,
    grid_manager: GridManager,
    disc_drawer: InstanceDrawer<GridDisc>,
    dna_drawers: DnaDrawers,
    direction_cube: InstanceDrawer<DirectionCube>,
    skybox_cube: InstanceDrawer<SkyBox>,
    fog_parameters: FogParameters,
    rendering_mode: RenderingMode,
    background3d: Background3D,
}

impl View {
    pub fn new(
        window_size: PhySize,
        area_size: PhySize,
        device: Rc<Device>,
        queue: Rc<Queue>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Self {
        let camera = Rc::new(RefCell::new(Camera::new(
            (0.0, 5.0, 10.0),
            Rotor3::identity(),
        )));
        let projection = Rc::new(RefCell::new(Projection::new(
            area_size.width,
            area_size.height,
            70f32.to_radians(),
            0.1,
            1000.0,
        )));
        let viewer = UniformBindGroup::new(
            device.clone(),
            queue.clone(),
            &Uniforms::from_view_proj(camera.clone(), projection.clone()),
        );
        let model_bg_desc = wgpu::BindGroupLayoutDescriptor {
            entries: MODEL_BG_ENTRY,
            label: None,
        };
        println!("Create letter drawer");
        let letter_drawer = BASIS_SYMBOLS
            .iter()
            .map(|c| {
                let letter = Letter::new(*c, device.clone(), queue.clone());
                InstanceDrawer::new(
                    device.clone(),
                    queue.clone(),
                    &viewer.get_layout_desc(),
                    &model_bg_desc,
                    letter,
                    false,
                )
            })
            .collect();
        println!("Create helix letter drawer");
        let helix_letter_drawer = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9']
            .iter()
            .map(|c| {
                let letter = Letter::new(*c, device.clone(), queue.clone());
                InstanceDrawer::new(
                    device.clone(),
                    queue.clone(),
                    &viewer.get_layout_desc(),
                    &model_bg_desc,
                    letter,
                    false,
                )
            })
            .collect();

        let depth_texture =
            texture::Texture::create_depth_texture(device.as_ref(), &area_size, SAMPLE_COUNT);
        let fake_depth_texture =
            texture::Texture::create_depth_texture(device.as_ref(), &window_size, 1);
        let msaa_texture = if SAMPLE_COUNT > 1 {
            Some(crate::utils::texture::Texture::create_msaa_texture(
                device.clone().as_ref(),
                &area_size,
                SAMPLE_COUNT,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ))
        } else {
            None
        };
        let models = DynamicBindGroup::new(device.clone(), queue.clone());

        let grid_textures = GridTextures::new(device.as_ref(), encoder);
        println!("Create grid drawer");

        let grid_drawer = InstanceDrawer::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
            grid_textures,
            false,
        );
        let grid_textures = GridTextures::new(device.as_ref(), encoder);
        let fake_grid_drawer = InstanceDrawer::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
            grid_textures,
            true,
        );
        let grid_manager = GridManager::new(grid_drawer, fake_grid_drawer);

        println!("Create disc  drawer");
        let disc_drawer = InstanceDrawer::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
            (),
            false,
        );

        println!("Create dna drawer");
        let dna_drawers = DnaDrawers::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
        );

        let direction_texture = DirectionTexture::new(device.clone(), queue.clone());
        let mut direction_cube = InstanceDrawer::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
            direction_texture,
            false,
        );
        direction_cube.new_instances(vec![Default::default()]);

        let mut skybox_cube = InstanceDrawer::new(
            device.clone(),
            queue.clone(),
            &viewer.get_layout_desc(),
            &model_bg_desc,
            (),
            false,
        );
        skybox_cube.new_instances(vec![SkyBox::new(500.)]);

        Self {
            camera,
            projection,
            depth_texture,
            fake_depth_texture,
            new_size: None,
            device: device.clone(),
            viewer,
            models,
            handle_drawers: HandlesDrawer::new(device.clone()),
            rotation_widget: RotationWidget::new(device),
            letter_drawer,
            helix_letter_drawer,
            redraw_twice: false,
            need_redraw: true,
            need_redraw_fake: true,
            draw_letter: false,
            msaa_texture,
            grid_manager,
            disc_drawer,
            dna_drawers,
            direction_cube,
            skybox_cube,
            fog_parameters: FogParameters::new(),
            rendering_mode: Default::default(),
            background3d: Default::default(),
        }
    }

    /// Notify the view of an update. According to the nature of this update, the view decides if
    /// it needs to be redrawn or not.
    pub fn update(&mut self, view_update: ViewUpdate) {
        self.need_redraw = true;
        match view_update {
            ViewUpdate::Size(size) => {
                self.new_size = Some(size);
                self.need_redraw_fake = true;
            }
            ViewUpdate::Camera => {
                self.viewer.update(&Uniforms::from_view_proj_fog(
                    self.camera.clone(),
                    self.projection.clone(),
                    &self.fog_parameters,
                ));
                self.handle_drawers
                    .update_camera(self.camera.clone(), self.projection.clone());
                self.need_redraw_fake = true;
                let dist = self.projection.borrow().cube_dist();
                self.direction_cube
                    .new_instances(vec![DirectionCube::new(dist)]);
            }
            ViewUpdate::Fog(fog) => {
                let fog_center = self.fog_parameters.alt_fog_center.clone();
                self.fog_parameters = fog;
                self.fog_parameters.alt_fog_center = fog_center;
                self.viewer.update(&Uniforms::from_view_proj_fog(
                    self.camera.clone(),
                    self.projection.clone(),
                    &self.fog_parameters,
                ));
            }
            ViewUpdate::Handles(descr) => {
                self.handle_drawers.update_decriptor(
                    descr,
                    self.camera.clone(),
                    self.projection.clone(),
                );
                self.need_redraw_fake = true;
            }

            ViewUpdate::RotationWidget(descr) => {
                self.rotation_widget.update_decriptor(
                    descr,
                    self.camera.clone(),
                    self.projection.clone(),
                );
                self.need_redraw_fake = true;
            }
            ViewUpdate::ModelMatrices(ref matrices) => {
                self.models.update(matrices.clone().as_slice());
                self.need_redraw_fake = true;
            }
            ViewUpdate::Letter(letter) => {
                for (i, instance) in letter.into_iter().enumerate() {
                    self.letter_drawer[i].new_instances(instance);
                }
            }
            ViewUpdate::GridLetter(letter) => {
                for (i, instance) in letter.into_iter().enumerate() {
                    self.helix_letter_drawer[i].new_instances(instance);
                }
            }
            ViewUpdate::Grids(grid) => self.grid_manager.new_instances(grid),
            ViewUpdate::GridDiscs(instances) => self.disc_drawer.new_instances(instances),
            ViewUpdate::RawDna(mesh, instances) => {
                self.dna_drawers
                    .get_mut(mesh)
                    .new_instances_raw(instances.as_ref());
                if let Some(mesh) = mesh.to_fake() {
                    let mut instances = instances.as_ref().clone();
                    for i in instances.iter_mut() {
                        if i.scale.z < 0.99 {
                            i.scale *= 2.5;
                        }
                    }
                    self.need_redraw_fake = true;
                    self.dna_drawers
                        .get_mut(mesh)
                        .new_instances_raw(instances.as_ref());
                }
                if let Some(mesh) = mesh.to_outline() {
                    self.dna_drawers
                        .get_mut(mesh)
                        .new_instances_raw(instances.as_ref());
                }
            }
            ViewUpdate::FogCenter(center) => {
                self.fog_parameters.alt_fog_center = center;
                self.viewer.update(&Uniforms::from_view_proj_fog(
                    self.camera.clone(),
                    self.projection.clone(),
                    &self.fog_parameters,
                ));
            }
        }
    }

    pub fn need_redraw_fake(&self) -> bool {
        self.need_redraw_fake
    }

    pub fn need_redraw(&self) -> bool {
        self.need_redraw | self.redraw_twice
    }

    /// Draw the scene
    pub fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        draw_type: DrawType,
        area: DrawArea,
        action_mode: ActionMode,
    ) {
        let fake_color = draw_type.is_fake();
        if let Some(size) = self.new_size.take() {
            self.depth_texture =
                Texture::create_depth_texture(self.device.as_ref(), &area.size, SAMPLE_COUNT);
            self.fake_depth_texture = Texture::create_depth_texture(self.device.as_ref(), &size, 1);
            self.msaa_texture = if SAMPLE_COUNT > 1 {
                Some(crate::utils::texture::Texture::create_msaa_texture(
                    self.device.clone().as_ref(),
                    &area.size,
                    SAMPLE_COUNT,
                    wgpu::TextureFormat::Bgra8UnormSrgb,
                ))
            } else {
                None
            };
        }
        let clear_color = if fake_color || self.background3d == Background3D::White {
            wgpu::Color {
                r: 1.,
                g: 1.,
                b: 1.,
                a: 1.,
            }
        } else {
            wgpu::Color {
                r: 0.,
                g: 0.,
                b: 0.,
                a: 1.,
            }
        };
        let viewer = &self.viewer;
        let viewer_bind_group = viewer.get_bindgroup();
        let viewer_bind_group_layout = viewer.get_layout();

        let attachment = if !fake_color {
            if let Some(ref msaa) = self.msaa_texture {
                msaa
            } else {
                target
            }
        } else {
            target
        };

        let resolve_target = if !fake_color && self.msaa_texture.is_some() {
            Some(target)
        } else {
            None
        };

        let depth_attachement = if !fake_color {
            &self.depth_texture
        } else {
            &self.fake_depth_texture
        };

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: true,
                    },
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: &depth_attachement.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: true,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: true,
                    }),
                }),
            });
            if fake_color {
                render_pass.set_viewport(
                    area.position.x as f32,
                    area.position.y as f32,
                    area.size.width as f32,
                    area.size.height as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_scissor_rect(
                    area.position.x,
                    area.position.y,
                    area.size.width,
                    area.size.height,
                );
            }

            if draw_type == DrawType::Design {
                for drawer in self.dna_drawers.fakes() {
                    drawer.draw(
                        &mut render_pass,
                        self.viewer.get_bindgroup(),
                        self.models.get_bindgroup(),
                    )
                }
            } else if draw_type == DrawType::Scene {
                if self.background3d == Background3D::Sky {
                    self.skybox_cube.draw(
                        &mut render_pass,
                        self.viewer.get_bindgroup(),
                        self.models.get_bindgroup(),
                    );
                }
                for drawer in self.dna_drawers.reals(self.rendering_mode) {
                    drawer.draw(
                        &mut render_pass,
                        self.viewer.get_bindgroup(),
                        self.models.get_bindgroup(),
                    )
                }
            } else if draw_type == DrawType::Phantom {
                for drawer in self.dna_drawers.phantoms() {
                    drawer.draw(
                        &mut render_pass,
                        self.viewer.get_bindgroup(),
                        self.models.get_bindgroup(),
                    )
                }
            } else if draw_type == DrawType::Grid {
                // Draw design elements and phantoms, to fill the depth buffer
                for drawer in self.dna_drawers.fakes_and_phantoms() {
                    drawer.draw(
                        &mut render_pass,
                        self.viewer.get_bindgroup(),
                        self.models.get_bindgroup(),
                    )
                }
            }

            if draw_type.wants_widget() {
                if action_mode.wants_handle() {
                    self.handle_drawers.draw(
                        &mut render_pass,
                        viewer_bind_group,
                        viewer_bind_group_layout,
                        fake_color,
                    );
                }

                if action_mode.wants_rotation() {
                    self.rotation_widget.draw(
                        &mut render_pass,
                        viewer_bind_group,
                        viewer_bind_group_layout,
                        fake_color,
                    );
                }
            }

            if !fake_color && self.draw_letter {
                for drawer in self.letter_drawer.iter_mut() {
                    drawer.draw(
                        &mut render_pass,
                        viewer_bind_group,
                        self.models.get_bindgroup(),
                    )
                }
            }

            if !fake_color {
                self.grid_manager.draw(
                    &mut render_pass,
                    viewer_bind_group,
                    self.models.get_bindgroup(),
                    false,
                );
                self.disc_drawer.draw(
                    &mut render_pass,
                    viewer_bind_group,
                    self.models.get_bindgroup(),
                );
                for drawer in self.helix_letter_drawer.iter_mut() {
                    drawer.draw(
                        &mut render_pass,
                        viewer_bind_group,
                        self.models.get_bindgroup(),
                    )
                }
            }

            if fake_color {
                self.need_redraw_fake = false;
            } else if self.redraw_twice {
                self.redraw_twice = false;
                self.need_redraw = true;
            } else {
                self.need_redraw = false;
            }
        }
        if !fake_color {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: &depth_attachement.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: true,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: true,
                    }),
                }),
            });
            render_pass.set_viewport(
                area.size.width as f32 / 20.,
                0.,
                (area.size.width as f32 / 10. * 1.5)
                    .max(100.)
                    .min(area.size.width as f32),
                (area.size.height as f32 / 10. * 1.5)
                    .max((100. * area.size.height as f32 / area.size.width as f32) as f32)
                    .min(area.size.height as f32),
                0.0,
                1.0,
            );
            self.direction_cube.draw(
                &mut render_pass,
                viewer_bind_group,
                self.models.get_bindgroup(),
            )
        } else if draw_type == DrawType::Grid {
            // render pass to draw the grids
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
                    attachment,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: true,
                    },
                }],
                // Reuse previous depth_stencil_attachment
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachmentDescriptor {
                    attachment: &depth_attachement.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    }),
                }),
            });
            render_pass.set_viewport(
                area.position.x as f32,
                area.position.y as f32,
                area.size.width as f32,
                area.size.height as f32,
                0.0,
                1.0,
            );
            render_pass.set_scissor_rect(
                area.position.x,
                area.position.y,
                area.size.width,
                area.size.height,
            );
            self.grid_manager.draw(
                &mut render_pass,
                viewer_bind_group,
                self.models.get_bindgroup(),
                true,
            );
        }
    }

    /// Get a pointer to the camera
    pub fn get_camera(&self) -> CameraPtr {
        self.camera.clone()
    }

    /// A pointer to the projection camera
    pub fn get_projection(&self) -> ProjectionPtr {
        self.projection.clone()
    }

    pub fn set_draw_letter(&mut self, value: bool) {
        self.draw_letter = value;
    }

    /// Compute the translation that needs to be applied to the objects affected by the handle
    /// widget.
    pub fn compute_translation_handle(
        &self,
        x_coord: f32,
        y_coord: f32,
        direction: HandleDir,
    ) -> Option<Vec3> {
        let (origin, dir) = self.handle_drawers.get_handle(direction)?;
        let (x0, y0) = self.handle_drawers.get_origin_translation()?;
        let p1 = unproject_point_on_line(
            origin,
            dir,
            self.camera.clone(),
            self.projection.clone(),
            x0,
            y0,
        )?;
        let p2 = unproject_point_on_line(
            origin,
            dir,
            self.camera.clone(),
            self.projection.clone(),
            x_coord,
            y_coord,
        )?;
        Some(p2 - p1)
    }

    /// Translate the widgets when the associated objects are translated.
    pub fn translate_widgets(&mut self, translation: Vec3) {
        self.need_redraw = true;
        self.handle_drawers.translate(translation);
        self.rotation_widget.translate(translation);
    }

    /// Initialise the rotation that will be applied on objects affected by the rotation widget.
    pub fn init_rotation(&mut self, x_coord: f32, y_coord: f32) {
        self.need_redraw = true;
        self.rotation_widget.init_rotation(x_coord, y_coord)
    }

    /// Initialise the translation that will be applied on objects affected by the handle widget.
    pub fn init_translation(&mut self, x: f32, y: f32) {
        self.need_redraw = true;
        self.handle_drawers.init_translation(x, y)
    }

    /// Compute the rotation that needs to be applied to the objects affected by the rotation
    /// widget.
    pub fn compute_rotation(
        &self,
        x: f32,
        y: f32,
        mode: RotationMode,
    ) -> Option<(Rotor3, Vec3, bool)> {
        self.rotation_widget.compute_rotation(
            x,
            y,
            self.camera.clone(),
            self.projection.clone(),
            mode,
        )
    }

    pub fn set_widget_candidate(&mut self, selected_id: Option<u32>) {
        self.redraw_twice |= self.rotation_widget.set_selected(selected_id);
        self.redraw_twice |= self.handle_drawers.set_selected(selected_id);
    }

    pub fn compute_projection_axis(
        &self,
        axis: &Axis,
        mouse_x: f64,
        mouse_y: f64,
    ) -> Option<isize> {
        let p1 = unproject_point_on_line(
            axis.origin,
            axis.direction,
            self.camera.clone(),
            self.projection.clone(),
            mouse_x as f32,
            mouse_y as f32,
        )?;

        let sign = (p1 - axis.origin).dot(axis.direction).signum();
        Some(((p1 - axis.origin).mag() * sign / axis.direction.mag()).round() as isize)
    }

    pub fn grid_intersection(&self, x_ndc: f32, y_ndc: f32) -> Option<GridIntersection> {
        let ray = maths_3d::cast_ray(x_ndc, y_ndc, self.camera.clone(), self.projection.clone());
        self.grid_manager.intersect(ray.0, ray.1)
    }

    pub fn set_candidate_grid(&mut self, grids: Vec<(usize, usize)>) {
        self.grid_manager.set_candidate_grid(grids)
    }

    pub fn set_selected_grid(&mut self, grids: Vec<(usize, usize)>) {
        self.grid_manager.set_selected_grid(grids)
    }

    pub fn rendering_mode(&mut self, mode: RenderingMode) {
        self.rendering_mode = mode;
        self.need_redraw = true;
    }

    pub fn background3d(&mut self, bg: Background3D) {
        self.background3d = bg;
        self.need_redraw = true;
    }
}

/// An notification to be given to the view
#[derive(Debug)]
pub enum ViewUpdate {
    /// The camera has moved and the view and projection matrix must be updated.
    Camera,
    /// The size of the drawing area has been modified
    Size(PhySize),
    /// The set of model matrices has been modified
    ModelMatrices(Vec<Mat4>),
    /// The set of phantom instances has been modified
    Handles(Option<HandlesDescriptor>),
    RotationWidget(Option<RotationWidgetDescriptor>),
    Letter(Vec<Vec<LetterInstance>>),
    GridLetter(Vec<Vec<LetterInstance>>),
    Grids(Rc<Vec<GridInstance>>),
    GridDiscs(Vec<GridDisc>),
    RawDna(Mesh, Rc<Vec<RawDnaInstance>>),
    Fog(FogParameters),
    FogCenter(Option<Vec3>),
}

#[derive(Eq, PartialEq, Debug, Copy, Clone, Hash)]
pub enum Mesh {
    Sphere,
    Tube,
    OutlineSphere,
    OutlineTube,
    FakeSphere,
    FakeTube,
    CandidateSphere,
    CandidateTube,
    SelectedSphere,
    SelectedTube,
    PhantomSphere,
    PhantomTube,
    FakePhantomTube,
    FakePhantomSphere,
    SuggestionSphere,
    SuggestionTube,
    PastedSphere,
    PastedTube,
    PivotSphere,
    XoverSphere,
    XoverTube,
    Prime3Cone,
    Prime3ConeOutline,
}

impl Mesh {
    fn to_fake(&self) -> Option<Self> {
        match self {
            Self::Sphere => Some(Self::FakeSphere),
            Self::Tube => Some(Self::FakeTube),
            Self::PhantomSphere => Some(Self::FakePhantomSphere),
            Self::PhantomTube => Some(Self::FakePhantomTube),
            _ => None,
        }
    }

    fn to_outline(&self) -> Option<Self> {
        match self {
            Self::Sphere => Some(Self::OutlineSphere),
            Self::Tube => Some(Self::OutlineTube),
            Self::Prime3Cone => Some(Self::Prime3ConeOutline),
            _ => None,
        }
    }
}

struct DnaDrawers {
    sphere: InstanceDrawer<SphereInstance>,
    tube: InstanceDrawer<TubeInstance>,
    outline_sphere: InstanceDrawer<SphereInstance>,
    outline_tube: InstanceDrawer<TubeInstance>,
    candidate_sphere: InstanceDrawer<SphereInstance>,
    candidate_tube: InstanceDrawer<TubeInstance>,
    selected_sphere: InstanceDrawer<SphereInstance>,
    selected_tube: InstanceDrawer<TubeInstance>,
    fake_sphere: InstanceDrawer<SphereInstance>,
    fake_tube: InstanceDrawer<TubeInstance>,
    phantom_sphere: InstanceDrawer<SphereInstance>,
    phantom_tube: InstanceDrawer<TubeInstance>,
    fake_phantom_sphere: InstanceDrawer<SphereInstance>,
    fake_phantom_tube: InstanceDrawer<TubeInstance>,
    suggestion_sphere: InstanceDrawer<SphereInstance>,
    suggestion_tube: InstanceDrawer<TubeInstance>,
    pasted_sphere: InstanceDrawer<SphereInstance>,
    pasted_tube: InstanceDrawer<TubeInstance>,
    pivot_sphere: InstanceDrawer<SphereInstance>,
    xover_sphere: InstanceDrawer<SphereInstance>,
    xover_tube: InstanceDrawer<TubeInstance>,
    prime3_cones: InstanceDrawer<dna_obj::ConeInstance>,
    outline_prime3_cones: InstanceDrawer<dna_obj::ConeInstance>,
}

impl DnaDrawers {
    pub fn get_mut(&mut self, key: Mesh) -> &mut dyn RawDrawer<RawInstance = RawDnaInstance> {
        match key {
            Mesh::Sphere => &mut self.sphere,
            Mesh::Tube => &mut self.tube,
            Mesh::OutlineSphere => &mut self.outline_sphere,
            Mesh::OutlineTube => &mut self.outline_tube,
            Mesh::CandidateSphere => &mut self.candidate_sphere,
            Mesh::CandidateTube => &mut self.candidate_tube,
            Mesh::SelectedSphere => &mut self.selected_sphere,
            Mesh::SelectedTube => &mut self.selected_tube,
            Mesh::PhantomSphere => &mut self.phantom_sphere,
            Mesh::PhantomTube => &mut self.phantom_tube,
            Mesh::FakeSphere => &mut self.fake_sphere,
            Mesh::FakeTube => &mut self.fake_tube,
            Mesh::FakePhantomSphere => &mut self.fake_phantom_sphere,
            Mesh::FakePhantomTube => &mut self.fake_phantom_tube,
            Mesh::SuggestionTube => &mut self.suggestion_tube,
            Mesh::SuggestionSphere => &mut self.suggestion_sphere,
            Mesh::PastedSphere => &mut self.pasted_sphere,
            Mesh::PastedTube => &mut self.pasted_tube,
            Mesh::PivotSphere => &mut self.pivot_sphere,
            Mesh::XoverSphere => &mut self.xover_sphere,
            Mesh::XoverTube => &mut self.xover_tube,
            Mesh::Prime3Cone => &mut self.prime3_cones,
            Mesh::Prime3ConeOutline => &mut self.outline_prime3_cones,
        }
    }

    pub fn reals(
        &mut self,
        rendering_mode: RenderingMode,
    ) -> Vec<&mut dyn RawDrawer<RawInstance = RawDnaInstance>> {
        let mut ret: Vec<&mut dyn RawDrawer<RawInstance = RawDnaInstance>> = vec![
            &mut self.sphere,
            &mut self.tube,
            &mut self.prime3_cones,
            &mut self.candidate_sphere,
            &mut self.candidate_tube,
            &mut self.selected_sphere,
            &mut self.selected_tube,
            &mut self.phantom_tube,
            &mut self.phantom_sphere,
            &mut self.suggestion_sphere,
            &mut self.suggestion_tube,
            &mut self.pasted_tube,
            &mut self.pasted_sphere,
            &mut self.pivot_sphere,
            &mut self.xover_sphere,
            &mut self.xover_tube,
        ];
        if rendering_mode == RenderingMode::Cartoon {
            ret.insert(3, &mut self.outline_tube);
            ret.insert(4, &mut self.outline_sphere);
            ret.insert(5, &mut self.outline_prime3_cones);
        }

        ret
    }

    pub fn fakes(&mut self) -> Vec<&mut dyn RawDrawer<RawInstance = RawDnaInstance>> {
        vec![&mut self.fake_sphere, &mut self.fake_tube]
    }

    pub fn phantoms(&mut self) -> Vec<&mut dyn RawDrawer<RawInstance = RawDnaInstance>> {
        vec![&mut self.fake_phantom_sphere, &mut self.fake_phantom_tube]
    }

    pub fn fakes_and_phantoms(&mut self) -> Vec<&mut dyn RawDrawer<RawInstance = RawDnaInstance>> {
        vec![
            &mut self.fake_sphere,
            &mut self.fake_tube,
            &mut self.fake_phantom_sphere,
            &mut self.fake_phantom_tube,
        ]
    }

    pub fn new(
        device: Rc<Device>,
        queue: Rc<Queue>,
        viewer_desc: &wgpu::BindGroupLayoutDescriptor<'static>,
        model_desc: &wgpu::BindGroupLayoutDescriptor<'static>,
    ) -> Self {
        Self {
            sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            prime3_cones: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            outline_sphere: InstanceDrawer::new_outliner(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
            ),
            outline_tube: InstanceDrawer::new_outliner(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
            ),
            outline_prime3_cones: InstanceDrawer::new_outliner(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
            ),
            candidate_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            candidate_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            suggestion_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            suggestion_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            xover_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            xover_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            pasted_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            pasted_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            selected_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            selected_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            pivot_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            phantom_sphere: InstanceDrawer::new_wireframe(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            phantom_tube: InstanceDrawer::new_wireframe(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                false,
            ),
            fake_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                true,
            ),
            fake_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                true,
            ),
            fake_phantom_sphere: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                true,
            ),
            fake_phantom_tube: InstanceDrawer::new(
                device.clone(),
                queue.clone(),
                viewer_desc,
                model_desc,
                (),
                true,
            ),
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DrawType {
    Scene,
    Design,
    Widget,
    Phantom,
    Grid,
}

impl DrawType {
    fn is_fake(&self) -> bool {
        *self != DrawType::Scene
    }

    fn wants_widget(&self) -> bool {
        match self {
            DrawType::Scene => true,
            DrawType::Design => false,
            DrawType::Widget => true,
            DrawType::Phantom => false,
            DrawType::Grid => false,
        }
    }
}
