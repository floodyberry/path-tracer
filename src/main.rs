// Based on smallpt, http://www.kevinbeason.com/smallpt/ which is also licensed under
// the MIT license.
use std::fs::File;
use std::io::BufWriter;
use std::io;
use std::io::prelude::*;
use std::sync::Arc;
use std::sync::mpsc::channel;

extern crate rand;
use rand::{Rng, XorShiftRng, SeedableRng};

extern crate threadpool;
use threadpool::ThreadPool;

extern crate num_cpus;

extern crate argparse;
use argparse::{ArgumentParser, Store};

mod path_tracer;
use path_tracer::*;

enum Material {
    Diffuse,
    Specular,
    Refractive
}

struct Sphere {
    material: Material,
    radius: f64,
    position: Vec3d,
    emission: Vec3d,
    colour: Vec3d
}

impl Sphere {
    fn new(material: Material, radius: f64, position: Vec3d, emission: Vec3d, colour: Vec3d) -> Sphere {
        Sphere {
          material: material, 
          radius: radius, 
          position: position,
          emission: emission,
          colour: colour
        }
    }
    fn intersect(&self, ray: &Ray) -> Option<f64> {
        let op = self.position - ray.origin;
        let b = op.dot(ray.direction);
        let determinant = b * b - op.dot(op) + self.radius * self.radius;
        if determinant < 0.0 { return None; }
        let determinant = determinant.sqrt();
        let t1 = b - determinant;
        let t2 = b + determinant;
        const EPSILON : f64 = 0.0001;
        if t1 > EPSILON {
            Some(t1)
        } else if t2 > EPSILON {
            Some(t2)
        } else {
            None
        }
    }
}

#[test]
fn intersection() {
    let sphere = Sphere::new(
        Material::Diffuse, 
        100.0, 
        Vec3d::new(0.0, 0.0, 200.0), 
        Vec3d::zero(), 
        Vec3d::zero());
    let ray = Ray::new(Vec3d::zero(), Vec3d::new(0.0, 0.0, 1.0));
    match sphere.intersect(&ray) {
        Some(x) => assert_eq!(x, 100.0),
        None => panic!("unexpected")
    }
    let ray = Ray::new(Vec3d::zero(), Vec3d::new(0.0, 1.0, 0.0));
    match sphere.intersect(&ray) {
        Some(_) => panic!("unexpected"),
        None => {}
    }
}

struct HitRecord<'a> {
    sphere: &'a Sphere,
    dist: f64
}

fn intersect<'a>(scene: &'a [Sphere], ray: &Ray) -> Option<HitRecord<'a>> {
    let mut result : Option<HitRecord<'a>> = None;
    for sph in scene {
        if let Some(dist) = sph.intersect(&ray) {
            if match result { None => true, Some(ref x) => dist < x.dist } {
                result = Some(HitRecord { sphere: &sph, dist: dist });
            }
        }
    }
    result
}

fn radiance<R: Rng>(scene: &[Sphere], ray: &Ray, depth: i32, rng: &mut R) -> Vec3d {
    if let Some(hit) = intersect(&scene, &ray) {
        let hit_pos = ray.origin + ray.direction * hit.dist;
        let hit_normal = (hit_pos - hit.sphere.position).normalized();
        let n1 = if hit_normal.dot(ray.direction) < 0.0 { hit_normal } else { hit_normal.neg() };
        let mut colour = hit.sphere.colour;
        let max_reflectance = colour.max_component();
        let depth = depth + 1;
        if depth > 5 {
            let rand = rng.gen::<f64>();
            if rand < max_reflectance && depth < 500 { // Rust's stack blows up ~600 on my machine
               colour = colour * (1.0 / max_reflectance);
            } else {
                return hit.sphere.emission;
            }
        }
        match hit.sphere.material {
            Material::Diffuse => {
                // Get a random polar coordinate
                let r1 = rng.gen::<f64>() * 2.0 * std::f64::consts::PI;
                let r2 = rng.gen::<f64>();
                let r2s = r2.sqrt();
                // Create a coordinate system u,v,w local to the point, where the w is the normal
                // pointing out of the sphere and the u and v are orthonormal to w.
                let w = n1;
                // Pick an arbitrary non-zero preferred axis for u
                let u = if n1.x.abs() > 0.1 { Vec3d::new(0.0, 1.0, 0.0) } else { Vec3d::new(1.0, 0.0, 0.0) }.cross(w);
                let v = w.cross(u);
                // construct the new direction
                let new_dir = u * r1.cos() * r2s + v * r1.sin() * r2s + w * (1.0 - r2).sqrt();
                colour = colour * radiance(scene, &Ray::new(hit_pos, new_dir.normalized()), depth, rng);
            }, 
            Material::Specular => {
                let reflection = ray.direction - hit_normal * 2.0 * hit_normal.dot(ray.direction);
                let reflected_ray = Ray::new(hit_pos, reflection);
                colour = colour * radiance(scene, &reflected_ray, depth, rng);
            },
            Material::Refractive => {
                let reflection = ray.direction - hit_normal * 2.0 * hit_normal.dot(ray.direction);
                let reflected_ray = Ray::new(hit_pos, reflection);
                let into = if hit_normal.dot(n1) > 0.0 { true } else { false };
                let nc = 1.0;
                let nt = 1.5;
                let nnt = if into { nc/nt } else { nt/nc };
                let ddn = ray.direction.dot(n1);
                let cos2t = 1.0 - nnt * nnt * (1.0 - ddn * ddn);
                if cos2t < 0.0 {
                    // Total internal reflection
                    colour = colour * radiance(scene, &reflected_ray, depth, rng);
                } else {
                    let tbd = ddn * nnt + cos2t.sqrt();
                    let tbd = if into { tbd } else { -tbd };
                    let tdir = (ray.direction * nnt - hit_normal * tbd).normalized();
                    let transmitted_ray = Ray::new(hit_pos, tdir);
                    let a = nt - nc;
                    let b = nt + nc;
                    let r0 = (a * a) / (b * b);
                    let c = 1.0 - if into { -ddn } else { tdir.dot(hit_normal) };
                    let re = r0 + (1.0 - r0) * c * c * c * c * c;
                    let tr = 1.0 - re;
                    let p = 0.25 + 0.5 * re;
                    let rp = re / p;
                    let tp = tr / (1.0 - p);
                    colour = colour * if depth > 2 {
                        if rng.gen::<f64>() < p {
                            radiance(scene, &reflected_ray, depth, rng) * rp
                        } else {
                            radiance(scene, &transmitted_ray, depth, rng) * tp
                        }
                    } else {
                        radiance(scene, &reflected_ray, depth, rng) * re +
                            radiance(scene, &transmitted_ray, depth, rng) * tr
                    }
                }
            }
        }
        hit.sphere.emission + colour
    } else {
        Vec3d::zero()
    }
}

fn random_samp<T: Rng>(rng: &mut T) -> f64 {
    let r = 2.0 * rng.gen::<f64>();
    if r < 1.0 { r.sqrt() - 1.0 } else { 1.0 - (2.0 - r).sqrt() }
}

fn to_int(v: f64) -> u8 {
    let ch = (v.powf(1.0/2.2) * 255.0 + 0.5) as i64;
    if ch < 0 { 0 } else if ch > 255 { 255 } else { ch as u8 }
}

fn main() {
    let mut samps = 1;
    let mut width = 1024;
    let mut height = 768;
    let mut output_filename = "image.ppm".to_string();
    {
        let mut ap = ArgumentParser::new();
        ap.set_description("Render a simple image");
        ap.refer(&mut samps).add_option(&["-s", "--samples"], Store, "Number of samples");
        ap.refer(&mut height).add_option(&["-h", "--height"], Store, "Height");
        ap.refer(&mut width).add_option(&["-w", "--width"], Store, "Width");
        ap.refer(&mut output_filename).add_option(&["-o", "--output"], Store, 
                                                  "Filename to output to");
        ap.parse_args_or_exit();
    }
    samps = samps / 4;
    if samps < 1 { samps = 1; }
    const BLACK : Vec3d = Vec3d { x: 0.0, y: 0.0, z: 0.0 };
    const RED : Vec3d = Vec3d { x: 0.75, y: 0.25, z: 0.25 };
    const BLUE : Vec3d = Vec3d { x: 0.25, y: 0.25, z: 0.75 };
    const GREY : Vec3d = Vec3d { x: 0.75, y: 0.75, z: 0.75 };
    const WHITE : Vec3d = Vec3d { x: 0.999, y: 0.999, z: 0.999 };
    let scene = Arc::new(vec!{
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(1e5+1.0, 40.8, 81.6),
            BLACK, RED),
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(-1e5+99.0, 40.8, 81.6),
            BLACK, BLUE),
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(50.0, 40.8, 1e5),
            BLACK, GREY),
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(50.0, 40.8, -1e5 + 170.0),
            BLACK, BLACK),
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(50.0, 1e5, 81.6),
            BLACK, GREY),
        Sphere::new(Material::Diffuse, 1e5, 
            Vec3d::new(50.0, -1e5 + 81.6, 81.6),
            BLACK, GREY),
        Sphere::new(Material::Specular, 16.5, 
            Vec3d::new(27.0, 16.5, 47.0),
            BLACK, WHITE),
        Sphere::new(Material::Refractive, 16.5, 
            Vec3d::new(73.0, 16.5, 78.0),
            BLACK, WHITE),
        Sphere::new(Material::Diffuse, 600.0, 
            Vec3d::new(50.0, 681.6 - 0.27, 81.6),
            Vec3d::new(12.0, 12.0, 12.0), BLACK),
    });

    let camera_pos = Vec3d::new(50.0, 52.0, 295.6);
    let camera_dir = Vec3d::new(0.0, -0.042612, -1.0);
    let camera_x = Vec3d::new(width as f64 * 0.5135 / height as f64, 0.0, 0.0);
    let camera_y = camera_x.cross(camera_dir).normalized() * 0.5135;

    let num_threads = num_cpus::get();
    println!("Using {} threads", num_threads);
    let pool = ThreadPool::new(num_threads);
    let (tx, rx) = channel();

    for y in 0..height {
        let tx = tx.clone();
        let scene = scene.clone();
        pool.execute(move || {
            let mut line = Vec::with_capacity(width);
            let mut rng = XorShiftRng::from_seed([1 + (y * y) as u32, 0x193a6754, 0x15aac60d, 0xb017f00d]);
            for x in 0..width {
                let mut sum = Vec3d::zero();
                for sx in 0..2 {
                    for sy in 0..2 {
                        for _samp in 0..samps {
                            let dx = random_samp(&mut rng);
                            let dy = random_samp(&mut rng);
                            let sub_x = (sx as f64 + 0.5 + dx) / 2.0;
                            let dir_x = (sub_x + x as f64) / width as f64 - 0.5;
                            let sub_y = (sy as f64 + 0.5 + dy) / 2.0;
                            let dir_y = (sub_y + (height -y - 1) as f64) / height as f64 - 0.5;
                            let dir = (camera_x * dir_x + camera_y * dir_y + camera_dir).normalized();
                            let jittered_ray = Ray::new(camera_pos + dir * 140.0, dir);
                            let sample = radiance(&scene, &jittered_ray, 0, &mut rng);
                            sum = sum + sample;
                        }
                    }
                }
                line.push((sum / (samps * 4) as f64).clamp());
            }
            tx.send((y, line)).unwrap();
        });
    }
    let mut left = height;
    let mut screen : Vec<Vec<Vec3d>> = Vec::new();
    for _y in 0..height {
        screen.push(Vec::new());
    }
    while left > 0 {
        print!("Rendering ({} spp) {:.4}%...\r", samps * 4, 100.0 * (height - left) as f64 / height as f64);
        io::stdout().flush().ok().expect("Could not flush stdout");
        let (y, line) = rx.recv().unwrap();
        screen[y] = line;
        left -= 1;
    }
    println!("\nWriting output to '{}'", output_filename);
    let output_file = File::create(output_filename).unwrap();
    let mut writer = BufWriter::new(output_file);
    write!(&mut writer, "P3\n{} {}\n255\n", width, height).unwrap();
    for y in 0..height {
        for x in 0..width {
            let sum = screen[y][x];
            write!(&mut writer, "{} {} {} ", to_int(sum.x), to_int(sum.y), to_int(sum.z)).unwrap();
        }
    }
}
