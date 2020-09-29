use futures::executor;
use iced_wgpu::wgpu;
use iced_winit::winit;
use std::collections::HashSet;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use ultraviolet::{Mat4, Rotor3, Vec3};

use crate::{design, utils, mediator};
use crate::{DrawArea, PhySize, WindowEvent};
use utils::{instance, BufferDimensions};
use instance::Instance;
use mediator::{MediatorPtr, Application, Notification, AppNotification};
use wgpu::{Device, Queue};
use winit::dpi::PhysicalPosition;

/// Computation of the view and projection matrix.
mod camera;
/// Display of the scene
mod view;
use view::{View, ViewUpdate};
/// Handling of inputs and notifications
mod controller;
use controller::{Consequence, Controller};
mod data;
use data::Data;
pub use data::SelectionMode;
pub use controller::ClickMode;
use design::{Design, DesignRotation, DesignTranslation, DesignNotification, DesignNotificationContent};

type ViewPtr = Rc<RefCell<View>>;
type DataPtr = Rc<RefCell<Data>>;

/// A structure responsible of the 3D display of the designs
pub struct Scene {
    device: Rc<Device>,
    queue: Rc<Queue>,
    /// The update to be performed before next frame
    update: SceneUpdate,
    /// The Object that handles the drawing to the 3d texture
    view: ViewPtr,
    /// The Object thant handles the designs data
    data: DataPtr,
    /// The Object that handles input and notifications
    controller: Controller,
    /// The limits of the area on which the scene is displayed
    area: DrawArea,
    pixel_to_check: Option<PhysicalPosition<f64>>,
    mediator: MediatorPtr,
}

impl Scene {
    /// Create a new scene.
    /// # Argument
    ///
    /// * `device` a reference to a `Device` object. This can be seen as a socket to the GPU
    ///
    /// * `queue` the command queue of `device`.
    ///
    /// * `window_size` the *Physical* size of the window in which the application is displayed
    ///
    /// * `area` the limits, in *physical* size of the area on which the scene is displayed
    pub fn new(device: Rc<Device>, queue: Rc<Queue>, window_size: PhySize, area: DrawArea, mediator: MediatorPtr) -> Self {
        let update = SceneUpdate::new();
        let view = Rc::new(RefCell::new(View::new(window_size, area.size, device.clone(), queue.clone())));
        let data = Rc::new(RefCell::new(Data::new(view.clone())));
        let controller = Controller::new(view.clone(), data.clone(), window_size, area.size);
        Self {
            device,
            queue,
            view,
            data,
            update,
            controller,
            area,
            pixel_to_check: None,
            mediator,
        }
    }

    /// Add a design to be rendered.
    pub fn add_design(&mut self, design: Arc<Mutex<Design>>) {
        self.data.borrow_mut().add_design(design)
    }

    /// Remove all designs
    pub fn clear_design(&mut self) {
        self.data.borrow_mut().clear_designs()
    }

    /// Return the list of designs selected
    fn get_selected_designs(&self) -> HashSet<u32> {
        self.data.borrow().get_selected_designs()
    }

    /// Input an event to the scene. Return true, if the selected object of the scene has changed
    pub fn input(
        &mut self,
        event: &WindowEvent,
        cursor_position: PhysicalPosition<f64>,
    ) {
        let camera_can_move = self.get_selected_designs().len() == 0;
        let consequence = self
            .controller
            .input(event, cursor_position, camera_can_move);
        match consequence {
            Consequence::Nothing => (),
            Consequence::CameraMoved => self.notify(SceneNotification::CameraMoved),
            Consequence::PixelSelected(clicked) => self.click_on(clicked),
            Consequence::Translation(x, y, z) => {
                self.translate_selected_design(x, y, z);
            }
            Consequence::MovementEnded => {
                self.mediator.lock().unwrap().notify_all_designs(AppNotification::MovementEnded);
            }
            Consequence::Rotation(x, y) => {
                let rotation = DesignRotation {
                    origin: self.get_selected_position().unwrap(),
                    up_vec: self.view.borrow().up_vec(),
                    right_vec: self.view.borrow().right_vec(),
                    angle_xz: x as f32 * std::f32::consts::PI,
                    angle_yz: y as f32 * std::f32::consts::PI,
                };
                self.mediator.lock().unwrap().notify_designs(&self.data.borrow().get_selected_designs(), AppNotification::Rotation(&rotation))
            }
            Consequence::Swing(x, y) => {
                let pivot = self.data.borrow().get_selected_position();
                if let Some(pivot) = pivot {
                    self.controller.set_pivot_point(pivot);
                    self.controller.swing(x, y);
                    self.notify(SceneNotification::CameraMoved);
                }
            }
            Consequence::CursorMoved(clicked) => {
                self.pixel_to_check = Some(clicked)
            }
        };
    }

    fn click_on(
        &mut self,
        clicked_pixel: PhysicalPosition<f64>,
    ) {
        let (selected_id, design_id) = self.set_selected_id(clicked_pixel);
        if selected_id != 0xFFFFFF {
            self.data.borrow_mut().set_selection(design_id, selected_id);
        } else {
            self.data.borrow_mut().reset_selection();
        }
        self.data.borrow_mut().notify_selection_update();
    }

    fn check_on(
        &mut self,
        clicked_pixel: PhysicalPosition<f64>,
    ) {
        let (checked_id, design_id) = self.set_selected_id(clicked_pixel);
        if checked_id != 0xFFFFFF {
            self.data.borrow_mut().set_candidate(design_id, checked_id);
        } else {
            self.data.borrow_mut().reset_candidate();
        }
        self.data.borrow_mut().notify_candidate_update();
    }

    fn set_selected_id(
        &mut self,
        clicked_pixel: PhysicalPosition<f64>,
    ) -> (u32, u32) {
        let size = wgpu::Extent3d {
            width: self.controller.get_window_size().width,
            height: self.controller.get_window_size().height,
            depth: 1,
        };

        let (texture, texture_view) = self.create_fake_scene_texture(self.device.as_ref(), size);

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        self.view
            .borrow_mut()
            .draw(&mut encoder, &texture_view, true, self.area);

        // create a buffer and fill it with the texture
        let extent = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth: 1,
        };
        let buffer_dimensions = BufferDimensions::new(extent.width as usize, extent.height as usize);
        let buf_size = buffer_dimensions.padded_bytes_per_row * buffer_dimensions.height;
        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            size: buf_size as u64,
            usage: wgpu::BufferUsage::MAP_READ | wgpu::BufferUsage::COPY_DST,
            mapped_at_creation: false,
            label: Some("staging_buffer"),
        });
        let buffer_copy_view = wgpu::BufferCopyView {
            buffer: &staging_buffer,
            layout: wgpu::TextureDataLayout {
                offset: 0,
                bytes_per_row: buffer_dimensions.padded_bytes_per_row as u32,
                rows_per_image: 0,
            },
        };
        let origin = wgpu::Origin3d {
           x: clicked_pixel.cast::<u32>().x, 
           y: clicked_pixel.cast::<u32>().y + self.area.position.y, 
           z: 0,
        };
        let texture_copy_view = wgpu::TextureCopyView {
            texture: &texture,
            mip_level: 0,
            origin,
        };

        encoder.copy_texture_to_buffer(texture_copy_view, buffer_copy_view, extent);
        self.queue.submit(Some(encoder.finish()));

        let pixel = 0;

        let buffer_slice = staging_buffer.slice(..);
        let buffer_future = buffer_slice.map_async(wgpu::MapMode::Read);
        self.device.poll(wgpu::Maintain::Wait);

        let future_color = async {
            if let Ok(()) = buffer_future.await {
                let pixels = buffer_slice.get_mapped_range();
                let a = pixels[pixel + 3] as u32;
                let r = (pixels[pixel + 2] as u32) << 16;
                let g = (pixels[pixel + 1] as u32) << 8;
                let b = pixels[pixel] as u32;
                let color = r + g + b;
                drop(pixels);
                staging_buffer.unmap();
                (color, a)
            } else {
                panic!("could not read fake texture");
            }
        };
        executor::block_on(future_color)
    }

    fn create_fake_scene_texture(&self, device: &Device, size: wgpu::Extent3d) -> (wgpu::Texture, wgpu::TextureView) {
        let desc = wgpu::TextureDescriptor {
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsage::OUTPUT_ATTACHMENT
                | wgpu::TextureUsage::SAMPLED
                | wgpu::TextureUsage::COPY_SRC,
            label: Some("desc"),
        };
        let texture_view_descriptor = wgpu::TextureViewDescriptor {
            label: Some("texture_view_descriptor"),
            format: Some(wgpu::TextureFormat::Bgra8Unorm),
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            level_count: None,
            base_array_layer: 0,
            array_layer_count: None,
        };

        let texture = device.create_texture(&desc);
        let view = texture.create_view(&texture_view_descriptor);
        (texture, view)
    }

    fn translate_selected_design(&mut self, x: f64, y: f64, z: f64) {
        let distance = (self.get_selected_position().unwrap() - self.camera_position())
            .dot(self.camera_direction())
            .abs()
            .sqrt();
        let height = 2. * distance * (self.get_fovy() / 2.).tan();
        let width = height * self.get_ratio();
        let right_vec = width * x as f32 * self.view.borrow().right_vec();
        let up_vec = height * -y as f32 * self.view.borrow().up_vec();
        let forward = z as f32 * self.view.borrow().get_camera_direction();
        let translation = DesignTranslation {
            right: right_vec,
            up: up_vec,
            forward,
        };
        self.mediator.lock().unwrap().notify_designs(&self.get_selected_designs(), AppNotification::Translation(&translation));
    }

    fn get_selected_position(&self) -> Option<Vec3> {
        self.data.borrow().get_selected_position()
    }

    /// Adapt the camera, position, orientation and pivot point to a design so that the design fits
    /// the scene, and the pivot point of the camera is the center of the design.
    pub fn fit_design(&mut self) {
        let camera = self.data.borrow().get_fitting_camera(self.get_ratio(), self.get_fovy());
        if let Some((position, rotor)) = camera {
            let pivot_point = self.data.borrow().get_middle_point(0).clone();
            self.controller
                .set_pivot_point(pivot_point);
            self.notify(SceneNotification::NewCamera(position, rotor));
        }
    }

    fn camera_position(&self) -> Vec3 {
        self.view.borrow().get_camera_position()
    }

    fn camera_direction(&self) -> Vec3 {
        self.view.borrow().get_camera_position()
    }

    /// Draw the scene
    pub fn draw_view(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        device: &Device,
        dt: Duration,
        fake_color: bool,
        queue: &Queue,
    ) {
        if let Some(pixel) = self.pixel_to_check.take() {
            self.check_on(pixel)
        }
        if self.controller.camera_is_moving() {
            self.notify(SceneNotification::CameraMoved);
        }
        if self.update.need_update {
            self.perform_update(dt);
        }

        self.view
            .borrow_mut()
            .draw(encoder, target, fake_color, self.area);
    }

    fn perform_update(&mut self, dt: Duration) {

        if self.update.camera_update {
            self.controller.update_camera(dt);
            self.view.borrow_mut().update(ViewUpdate::Camera);
            self.update.camera_update = false;
        }
        self.update.need_update = false;
    }


    /// Return the vertical field of view of the camera in radians
    pub fn get_fovy(&self) -> f32 {
        self.view.borrow().get_projection().borrow().get_fovy()
    }

    /// Return the width/height ratio of the camera
    pub fn get_ratio(&self) -> f32 {
        self.view.borrow().get_projection().borrow().get_ratio()
    }

    pub fn change_selection_mode(&mut self, selection_mode: SelectionMode) {
        self.data.borrow_mut().change_selection_mode(selection_mode)
    }
}

/// A structure that stores the element that needs to be updated in a scene
pub struct SceneUpdate {
    pub tube_instances: Option<Vec<Instance>>,
    pub sphere_instances: Option<Vec<Instance>>,
    pub fake_tube_instances: Option<Vec<Instance>>,
    pub fake_sphere_instances: Option<Vec<Instance>>,
    pub selected_tube: Option<Vec<Instance>>,
    pub selected_sphere: Option<Vec<Instance>>,
    pub candidate_spheres: Option<Vec<Instance>>,
    pub candidate_tubes: Option<Vec<Instance>>,
    pub model_matrices: Option<Vec<Mat4>>,
    pub need_update: bool,
    pub camera_update: bool,
}

impl SceneUpdate {
    pub fn new() -> Self {
        Self {
            tube_instances: None,
            sphere_instances: None,
            fake_tube_instances: None,
            fake_sphere_instances: None,
            selected_tube: None,
            selected_sphere: None,
            candidate_spheres: None,
            candidate_tubes: None,
            need_update: false,
            camera_update: false,
            model_matrices: None,
        }
    }
}

/// A notification to be given to the scene
pub enum SceneNotification {
    /// The camera has moved. As a consequence, the projection and view matrix must be
    /// updated.
    CameraMoved,
    /// The camera is replaced by a new one. 
    NewCamera(Vec3, Rotor3),
    /// The drawing area has been modified
    NewSize(PhySize, DrawArea),
}

impl Scene {
    /// Send a notificatoin to the scene
    pub fn notify(&mut self, notification: SceneNotification) {
        match notification {
            SceneNotification::NewCamera(position, projection) => {
                self.controller.teleport_camera(position, projection);
                self.update.camera_update = true;
            }
            SceneNotification::CameraMoved => self.update.camera_update = true,
            SceneNotification::NewSize(window_size, area) => {
                self.area = area;
                self.resize(window_size);
            }
        };
        self.update.need_update = true;
    }

    fn resize(&mut self, window_size: PhySize) {
        self.view.borrow_mut().update(ViewUpdate::Size(window_size));
        self.controller.resize(window_size, self.area.size);
        self.update.camera_update = true;
    }
}

impl Application for Scene {
    fn on_notify(&mut self, notification: Notification) {
        match notification {
            Notification::DesignNotification(notification) => self.handle_design_notification(notification),
            Notification::AppNotification(_) => (),
            Notification::NewDesign(design) => {
                self.add_design(design)
            }
            Notification::ClearDesigns => {
                self.clear_design()
            }
        }
    }
}

impl Scene {
    fn handle_design_notification(&mut self, notification: DesignNotification) {
        let design_id = notification.design_id;
        match notification.content {
            DesignNotificationContent::ModelChanged(matrix) => {
                self.update.need_update = true;
                self.view.borrow_mut().update_model_matrix(design_id, matrix)
            }
        }
    }
}
