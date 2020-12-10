use super::ViewPtr;
use crate::design::{Design, StrandBuilder};
use std::sync::{Arc, Mutex};
use ultraviolet::Vec2;

mod helix;
pub use helix::{GpuVertex, Helix, HelixModel};
mod strand;
pub use strand::{FreeEnd, Nucl, Strand, StrandVertex};
mod design;
use super::CameraPtr;
use crate::consts::*;
use crate::utils::camera2d::FitRectangle;
use design::{Design2d, Helix2d};

pub struct Data {
    view: ViewPtr,
    design: Design2d,
    instance_update: bool,
    instance_reset: bool,
    helices: Vec<Helix>,
    selected_helix: Option<usize>,
}

impl Data {
    pub fn new(view: ViewPtr, design: Arc<Mutex<Design>>) -> Self {
        Self {
            view,
            design: Design2d::new(design),
            instance_update: true,
            instance_reset: false,
            helices: Vec::new(),
            selected_helix: None,
        }
    }

    pub fn perform_update(&mut self) {
        if self.instance_reset {
            self.view.borrow_mut().reset();
            self.instance_reset = false;
        }
        if self.instance_update {
            self.design.update();
            self.fetch_helices();
            self.view.borrow_mut().update_helices(&self.helices);
            self.view
                .borrow_mut()
                .update_strands(&self.design.get_strands(), &self.helices);
        }
        self.instance_update = false;
    }

    fn fetch_helices(&mut self) {
        let nb_helix = self.helices.len();
        let new_helices = self.design.get_helices();
        for (i, helix) in self.helices.iter_mut().enumerate() {
            helix.update(&new_helices[i]);
        }
        for (delta, h) in new_helices[nb_helix..].iter().enumerate() {
            self.helices.push(Helix::new(
                h.left,
                h.right,
                (5. * (delta + nb_helix) as f32 - 1.) * Vec2::unit_y(),
                (delta + nb_helix) as u32,
            ))
        }
    }

    pub fn get_click(&self, x: f32, y: f32, camera: &CameraPtr) -> ClickResult {
        for h in self.helices.iter() {
            if h.click_on_circle(x, y, camera) {
                let translation_pivot = h.get_circle_pivot(camera).unwrap();
                return ClickResult::CircleWidget { translation_pivot };
            }
        }
        for (h_id, h) in self.helices.iter().enumerate() {
            let ret = h.get_click(x, y).map(|(position, forward)| Nucl {
                helix: h_id,
                position,
                forward,
            });
            if let Some(ret) = ret {
                return ClickResult::Nucl(ret);
            }
        }
        ClickResult::Nothing
    }

    pub fn get_rotation_pivot(&self, h_id: usize, camera: &CameraPtr) -> Option<Vec2> {
        self.helices
            .get(h_id)
            .and_then(|h| h.visible_center(camera))
    }

    pub fn get_click_unbounded_helix(&self, x: f32, y: f32, h_id: usize) -> Nucl {
        let (position, forward) = self.helices[h_id].get_click_unbounded(x, y);
        Nucl {
            position,
            forward,
            helix: h_id,
        }
    }

    pub fn get_pivot_position(&self, helix: usize, position: isize) -> Option<Vec2> {
        self.helices.get(helix).map(|h| h.get_pivot(position))
    }

    pub fn set_selected_helix(&mut self, helix: Option<usize>) {
        if let Some(h) = self.selected_helix {
            self.helices[h].set_color(HELIX_BORDER_COLOR);
        }
        self.selected_helix = helix;
        if let Some(h) = helix {
            self.helices[h].set_color(0xFF_BF_1E_28);
        }
        self.instance_update = true;
    }

    pub fn snap_helix(&mut self, pivot: Nucl, destination: Vec2) {
        if let Some(h) = self.selected_helix {
            self.helices[h].snap(pivot, destination);
            self.instance_update = true;
        }
    }

    pub fn rotate_helix(&mut self, pivot: Vec2, angle: f32) {
        if let Some(h) = self.selected_helix {
            self.helices[h].rotate(pivot, angle);
            self.instance_update = true;
        }
    }

    pub fn end_movement(&mut self) {
        for h in self.helices.iter_mut() {
            h.end_movement()
        }
    }

    pub fn move_helix_forward(&mut self) {
        if let Some(helix) = self.selected_helix {
            self.helices[helix].move_forward();
            self.instance_update = true;
        }
    }

    pub fn move_helix_backward(&mut self) {
        if let Some(helix) = self.selected_helix {
            self.helices[helix].move_backward();
            self.instance_update = true;
        }
    }

    pub fn helix_id_design(&self, id: usize) -> usize {
        self.design.get_helices()[id].id
    }

    pub fn get_builder(&self, nucl: Nucl, stick: bool) -> Option<StrandBuilder> {
        let real_helix = self.design.get_helices()[nucl.helix].id;
        self.design.get_builder(
            Nucl {
                helix: real_helix,
                ..nucl
            },
            stick,
        )
    }

    pub fn notify_update(&mut self) {
        self.instance_update = true;
    }

    pub fn merge_strand(&mut self, prime5: usize, prime3: usize) {
        self.instance_reset = true;
        self.instance_update = true;
        self.design.merge_strand(prime5, prime3)
    }

    pub fn can_cross_to(&self, from: Nucl, to: Nucl) -> bool {
        let from = self.to_real(from);
        let to = self.to_real(to);
        let prim5 = self.design.prime5_of(from).or(self.design.prime5_of(to));
        let prim3 = self.design.prime3_of(from).or(self.design.prime3_of(to));
        prim3.zip(prim5).is_some()
    }

    /// Return Some(true) if nucl is a 3' end, Some(false) if nucl is a 5' end and None otherwise
    pub fn is_strand_end(&self, nucl: Nucl) -> Option<bool> {
        let nucl = self.to_real(nucl);
        self.design
            .prime3_of(nucl)
            .map(|_| true)
            .or(self.design.prime5_of(nucl).map(|_| false))
    }

    pub fn set_free_end(&mut self, free_end: Option<FreeEnd>) {
        self.view.borrow_mut().set_free_end(free_end);
        self.view
            .borrow_mut()
            .update_strands(&self.design.get_strands(), &self.helices);
    }

    pub fn xover(&mut self, from: Nucl, to: Nucl) {
        let nucl1 = self.to_real(from);
        let nucl2 = self.to_real(to);
        let prim5 = self
            .design
            .prime5_of(nucl1)
            .or(self.design.prime5_of(nucl2))
            .unwrap();
        let prim3 = self
            .design
            .prime3_of(nucl1)
            .or(self.design.prime3_of(nucl2))
            .unwrap();
        self.merge_strand(prim3, prim5)
    }

    pub fn split_strand(&mut self, nucl: Nucl) {
        let nucl = self.to_real(nucl);
        self.instance_reset = true;
        self.design.split_strand(nucl);
    }

    fn to_real(&self, nucl: Nucl) -> Nucl {
        let real_helix = self.design.get_helices()[nucl.helix].id;
        Nucl {
            helix: real_helix,
            ..nucl
        }
    }

    pub fn get_fit_rectangle(&self) -> FitRectangle {
        let mut ret = FitRectangle {
            min_x: -5.,
            max_x: 15.,
            min_y: -30.,
            max_y: 5.,
        };
        for h in self.helices.iter() {
            let left = h.get_pivot(h.get_left());
            ret.add_point(Vec2::new(left.x, -left.y));
            let right = h.get_pivot(h.get_right());
            ret.add_point(Vec2::new(right.x, -right.y));
        }
        ret
    }
}

#[derive(Debug, PartialEq)]
pub enum ClickResult {
    Nucl(Nucl),
    CircleWidget { translation_pivot: Nucl },
    Nothing,
}
