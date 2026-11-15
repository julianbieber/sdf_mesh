import package::sdf_types::Skin;

struct Surface {
    dist: f32,
    weights: vec4<f32>,
    skin: Skin,
}

struct Joint {
    translation: vec3<f32>,
    rotation: vec4<f32>,
}

fn root_skin() -> Skin {
    return Skin(vec4(0u), vec4(1.0, 0.0, 0.0, 0.0));
}

fn surface_blend(dist: f32, weights: vec4<f32>) -> Surface {
    return Surface(dist, weights, root_skin());
}

fn surface(dist: f32, material: u32) -> Surface {
    return surface_blend(dist, vec4(
        select(0.0, 1.0, material == 0u),
        select(0.0, 1.0, material == 1u),
        select(0.0, 1.0, material == 2u),
        select(0.0, 1.0, material >= 3u),
    ));
}

fn on_joint(s: Surface, joint: u32) -> Surface {
    var out = s;
    out.skin = Skin(vec4(joint, 0u, 0u, 0u), vec4(1.0, 0.0, 0.0, 0.0));
    return out;
}

fn skin_mix(x: Skin, y: Skin, t: f32) -> Skin {
    var ids = array<u32, 8>(
        y.joints.x, y.joints.y, y.joints.z, y.joints.w,
        x.joints.x, x.joints.y, x.joints.z, x.joints.w,
    );
    var ws = array<f32, 8>(
        y.weights.x * t, y.weights.y * t, y.weights.z * t, y.weights.w * t,
        x.weights.x * (1.0 - t), x.weights.y * (1.0 - t),
        x.weights.z * (1.0 - t), x.weights.w * (1.0 - t),
    );
    for (var i = 0u; i < 8u; i = i + 1u) {
        for (var j = i + 1u; j < 8u; j = j + 1u) {
            if ids[j] == ids[i] {
                ws[i] = ws[i] + ws[j];
                ws[j] = 0.0;
            }
        }
    }
    var out = Skin(vec4(0u), vec4(0.0));
    for (var k = 0u; k < 4u; k = k + 1u) {
        var best = 0u;
        for (var i = 1u; i < 8u; i = i + 1u) {
            if ws[i] > ws[best] {
                best = i;
            }
        }
        out.joints[k] = ids[best];
        out.weights[k] = max(ws[best], 0.0);
        ws[best] = -1.0;
    }
    let total = out.weights.x + out.weights.y + out.weights.z + out.weights.w;
    if total <= 0.0 {
        return root_skin();
    }
    out.weights = out.weights / total;
    return out;
}

fn joint(translation: vec3<f32>, rotation: vec4<f32>) -> Joint {
    return Joint(translation, rotation);
}

fn joint_identity() -> Joint {
    return Joint(vec3(0.0), vec4(0.0, 0.0, 0.0, 1.0));
}

fn quat_axis_angle(axis: vec3<f32>, angle: f32) -> vec4<f32> {
    return vec4(normalize(axis) * sin(0.5 * angle), cos(0.5 * angle));
}

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn op_joint(p: vec3<f32>, j: Joint) -> vec3<f32> {
    let q = normalize(j.rotation);
    return quat_rotate(vec4(-q.xyz, q.w), p - j.translation);
}

fn sd_sphere(p: vec3<f32>, radius: f32) -> f32 {
    return length(p) - radius;
}

fn sd_box(p: vec3<f32>, half_extents: vec3<f32>) -> f32 {
    let q = abs(p) - half_extents;
    return length(max(q, vec3(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sd_round_box(p: vec3<f32>, half_extents: vec3<f32>, radius: f32) -> f32 {
    return sd_box(p, half_extents - vec3(radius)) - radius;
}

fn sd_cylinder(p: vec3<f32>, half_height: f32, radius: f32) -> f32 {
    let d = abs(vec2(length(p.xz), p.y)) - vec2(radius, half_height);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0)));
}

fn sd_torus(p: vec3<f32>, major_radius: f32, minor_radius: f32) -> f32 {
    let q = vec2(length(p.xz) - major_radius, p.y);
    return length(q) - minor_radius;
}

fn sd_plane(p: vec3<f32>, normal: vec3<f32>, offset: f32) -> f32 {
    return dot(p, normalize(normal)) + offset;
}

fn sd_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, radius: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - radius;
}

fn op_union(a: Surface, b: Surface) -> Surface {
    if a.dist <= b.dist {
        return a;
    }
    return b;
}

fn op_intersect(a: Surface, b: Surface) -> Surface {
    if a.dist >= b.dist {
        return a;
    }
    return b;
}

fn op_subtract(a: Surface, b: Surface) -> Surface {
    if a.dist >= -b.dist {
        return a;
    }
    var out = a;
    out.dist = -b.dist;
    return out;
}

fn op_smooth_union(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 + 0.5 * (b.dist - a.dist) / k, 0.0, 1.0);
    let dist = mix(b.dist, a.dist, h) - k * h * (1.0 - h);
    return Surface(dist, mix(b.weights, a.weights, h), skin_mix(b.skin, a.skin, h));
}

fn op_smooth_intersect(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 - 0.5 * (b.dist - a.dist) / k, 0.0, 1.0);
    let dist = mix(b.dist, a.dist, h) + k * h * (1.0 - h);
    return Surface(dist, mix(b.weights, a.weights, h), skin_mix(b.skin, a.skin, h));
}

fn op_smooth_subtract(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 - 0.5 * (a.dist + b.dist) / k, 0.0, 1.0);
    let dist = mix(a.dist, -b.dist, h) + k * h * (1.0 - h);
    var out = a;
    out.dist = dist;
    return out;
}

fn op_translate(p: vec3<f32>, offset: vec3<f32>) -> vec3<f32> {
    return p - offset;
}

fn op_repeat(p: vec3<f32>, period: vec3<f32>) -> vec3<f32> {
    return p - period * round(p / period);
}

fn op_rotate_y(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(c * p.x - s * p.z, p.y, s * p.x + c * p.z);
}

fn op_rotate_x(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(p.x, c * p.y - s * p.z, s * p.y + c * p.z);
}

fn op_rotate_z(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(c * p.x - s * p.y, s * p.x + c * p.y, p.z);
}

fn op_mirror_x(p: vec3<f32>) -> vec3<f32> {
    return vec3(abs(p.x), p.y, p.z);
}

fn op_twist_y(p: vec3<f32>, amount: f32) -> vec3<f32> {
    return op_rotate_y(p, amount * p.y);
}
