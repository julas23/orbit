//! The wallpaper effects that are scenes (`effects`): a black hole, a
//! beach at night, and the sky outside. Each is a pixel shader after
//! `effects::HEADER`.

/// After Interstellar's Gargantua: a thin disk of hot gas seen almost edge
/// on, its far side lifted over the hole and its underside showing below
/// it, both bent by the hole's gravity; the photon ring; the stars behind
/// bent round it. Each pixel's ray is followed back past the hole (in
/// units of its radius), and the disk is lit where the ray crosses it.
pub const BLACKHOLE: &str = r#"
// The stars on the sky's sphere, seen three ways and blended (no seam
// for the hole to bend into a ring), and a faint Milky Way.
vec3 backdrop(vec3 d) {
    vec3 w = pow(abs(d), vec3(6.0));
    w /= w.x + w.y + w.z;
    float s = w.x * (stars(d.yz, 70.0, 0.9) + 0.7 * stars(d.yz + 7.3, 150.0, 0.95))
        + w.y * (stars(d.zx, 70.0, 0.9) + 0.7 * stars(d.zx + 7.3, 150.0, 0.95))
        + w.z * (stars(d.xy, 70.0, 0.9) + 0.7 * stars(d.xy + 7.3, 150.0, 0.95));
    vec3 c = c0 + vec3(s);
    float band = d.y - 0.35 * d.x;
    c += c1 * 0.35 * exp(-band * band * 12.0) * fbm(d.xz * 4.0 + 3.0);
    return c;
}

vec2 turn(vec2 p, float a) {
    float s = sin(a);
    float k = cos(a);
    return vec2(k * p.x - s * p.y, s * p.x + k * p.y);
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 p = (v_coords - 0.5) * vec2(aspect, -1.0);
    // The camera a little above the disk's plane, drifting slowly round.
    float tilt = 0.11 + 0.03 * sin(time * 0.05);
    vec3 cam = vec3(0.0, sin(tilt), -cos(tilt)) * 24.0;
    cam.xz = turn(cam.xz, time * 0.01);
    vec3 fwd = normalize(-cam);
    vec3 right = normalize(cross(fwd, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(right, fwd);
    vec3 v = normalize(fwd * 1.55 + p.x * right + p.y * up);
    vec3 pos = cam;
    vec3 h = cross(pos, v);
    float h2 = dot(h, h);
    vec3 col = vec3(0.0);
    float through = 1.0;
    float spin = time * 0.9 * (1.0 + 0.6 * bass);
    for (int i = 0; i < 160; i++) {
        float r = length(pos);
        if (r < 1.0) {
            through = 0.0;
            break;
        }
        if (r > 45.0 && dot(pos, v) > 0.0) break;
        float dt = clamp(0.07 * r, 0.035, 2.0);
        vec3 prev = pos;
        v += -1.5 * h2 * pos / pow(r, 5.0) * dt;
        pos += v * dt;
        if (prev.y * pos.y < 0.0) {
            vec3 at = mix(prev, pos, prev.y / (prev.y - pos.y));
            float rr = length(at.xz);
            if (rr > 2.4 && rr < 15.0) {
                // The gas goes round as the planets do: faster inside.
                vec2 q = turn(at.xz, -spin / pow(rr, 1.5));
                float grain = fbm(q * 1.1 + rr * 0.35) * 0.7 + 0.45 * noise(vec2(rr * 5.0, 0.0));
                float dense = smoothstep(2.4, 3.2, rr) * smoothstep(15.0, 6.0, rr) * (0.35 + grain);
                float heat = clamp((15.0 - rr) / 12.0, 0.0, 1.0);
                vec3 gas = lit(heat * heat) * (0.6 + 1.6 * heat * heat);
                // Brighter where the gas comes towards us.
                vec3 flow = normalize(vec3(-at.z, 0.0, at.x));
                float doppler = 1.0 + 0.3 * dot(flow, -normalize(v));
                float a = clamp(dense * 1.3, 0.0, 1.0);
                col += through * a * gas * doppler * doppler * doppler * (1.3 + treble);
                through *= 1.0 - a * 0.9;
                if (through < 0.02) break;
            }
        }
    }
    col += through * backdrop(normalize(v));
    // A soft bloom, the eye's.
    col = col / (1.0 + 0.25 * col);
    finish(clamp(col, 0.0, 1.0));
}
"#;

/// A beach at night: the moon low over the sea and its path on the water,
/// the waves foaming up the sand; on the right a headland, its trees
/// against the sky, a waterfall pouring down its face into a pool and its
/// mist; palms leaning over the beach on the left; fireflies.
pub const BEACH: &str = r#"
const float PI = 3.14159265;

// Distance from `p` to the segment `a`–`b`, and how far along it (0..1).
vec2 segment(vec2 p, vec2 a, vec2 b) {
    vec2 ab = b - a;
    float s = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
    return vec2(length(p - a - ab * s), s);
}

// A palm, how much of `p` it covers (0..1): a trunk bending from `foot` to
// `crown`, fronds arching out and drooping from the crown, swaying.
float palm(vec2 p, vec2 foot, vec2 crown, float seed, float px) {
    float cover = 0.0;
    // The trunk, a curve through a point pushed sideways, thinning upwards.
    vec2 bend = mix(foot, crown, 0.5) + vec2(0.06 * sign(crown.x - foot.x), -0.02);
    vec2 prev = foot;
    for (int i = 1; i <= 8; i++) {
        float t = float(i) / 8.0;
        vec2 q = mix(mix(foot, bend, t), mix(bend, crown, t), t);
        vec2 d = segment(p, prev, q);
        float along = (float(i) - 1.0 + d.y) / 8.0;
        float w = mix(0.011, 0.0055, along) * (0.9 + 0.1 * sin(along * 140.0));
        cover = max(cover, smoothstep(w + px, w - px, d.x));
        prev = q;
    }
    if (length(p - crown) > 0.26) return cover;
    float sway = 0.05 * sin(time * 0.6 + seed) + 0.02 * sin(time * 1.7 + seed * 3.0);
    for (int k = 0; k < 11; k++) {
        float fk = float(k);
        // Spread round the crown, most upwards and sideways, two hanging.
        float a = k < 9 ? mix(-0.35, PI + 0.35, fk / 8.0) : -PI * 0.5 + (fk - 9.5) * 0.9;
        a += 0.22 * (fract(seed * 13.7 + fk * 0.61) - 0.5) + sway;
        float len = (k < 9 ? 0.16 : 0.09) + 0.05 * fract(seed * 7.3 + fk * 0.37);
        vec2 dir = vec2(cos(a), sin(a));
        vec2 last = crown;
        for (int j = 1; j <= 8; j++) {
            float s = float(j) / 8.0;
            // Out along `dir`, arching over and falling at the tip.
            vec2 q = crown + dir * len * s + vec2(0.0, -1.1 * len * s * s);
            vec2 d = segment(p, last, q);
            float at = (float(j) - 1.0 + d.y) / 8.0;
            float w = 0.017 * sin(PI * min(at * 1.1 + 0.08, 1.0)) * (0.65 + 0.35 * abs(sin(at * 85.0 + fk)));
            cover = max(cover, smoothstep(w + px, w - px, d.x));
            last = q;
        }
    }
    return max(cover, smoothstep(0.016 + px, 0.016 - px, length(p - crown)));
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, 1.0 - v_coords.y);
    float px = 1.5 / size.y;
    float t = time;
    float H = 0.44;
    vec2 moon = vec2(aspect * 0.36, 0.76);
    vec3 night = c0;
    vec3 low = c1 * 1.25 + vec3(0.015, 0.025, 0.04);
    vec3 light = c2;

    // The sky, a glow above the sea; the moon and its halo; stars; thin
    // clouds drifting across, lit near the moon.
    vec3 c = mix(low, night, pow(smoothstep(H, 1.0, uv.y), 0.7));
    float dm = length(uv - moon);
    c += light * (0.22 * exp(-dm * 3.5) + 0.25 * exp(-dm * 16.0));
    float wisp = fbm(vec2(uv.x * 1.4 - t * 0.008, uv.y * 7.0)) * smoothstep(H + 0.05, H + 0.3, uv.y);
    wisp = smoothstep(0.5, 0.85, wisp);
    c += vec3(stars(uv, 160.0, 0.93) + stars(uv + 2.7, 60.0, 0.985)) * smoothstep(H, H + 0.3, uv.y)
        * (1.0 - wisp) * smoothstep(0.05, 0.3, dm);
    c = mix(c, mix(low * 0.8, light * 0.55, exp(-dm * 4.0)), wisp * 0.6);
    float disc = smoothstep(0.036 + px, 0.036 - px, dm);
    vec3 face = light * (0.92 + 0.12 * fbm((uv - moon) * 40.0 + 3.0)) * 1.05;
    c = mix(c, face, disc);

    // Islands far out, pale with the distance.
    float isle = H + 0.022 * exp(-pow((uv.x - aspect * 0.12) / 0.22, 2.0)) + 0.012 * exp(-pow((uv.x - aspect * 0.5) / 0.08, 2.0));
    isle += 0.004 * fbm(vec2(uv.x * 30.0, 1.0));
    c = mix(c, mix(low, night, 0.35) * 0.85, smoothstep(isle + px, isle - px, uv.y) * step(H, uv.y));

    // The sea: darker near, the moon's path broken by the waves.
    // The beach curves away into the sea before the headland: a cove.
    float shore = 0.25 + 0.04 * sin(uv.x * 1.3 + 0.4) - 0.2 * smoothstep(aspect * 0.42, aspect * 0.72, uv.x);
    if (uv.y < H) {
        float near = (H - uv.y) / H;
        float z = 1.0 / (H - uv.y + 0.015);
        vec3 sea = mix(low * 0.75, night * 0.7, smoothstep(0.0, 0.7, near));
        float waves = noise(vec2(uv.x * z * 1.1 + t * 0.05, z * 2.2 - t * 0.5))
            * 0.6 + 0.4 * noise(vec2(uv.x * z * 2.3 - t * 0.04, z * 4.0 - t * 0.8));
        float path = exp(-pow((uv.x - moon.x) / (0.015 + 0.3 * (H - uv.y)), 2.0));
        sea += light * (smoothstep(0.55, 0.85, waves) * path * 0.9 + path * 0.06 * (1.0 - near));
        sea += light * 0.025 * waves;
        c = sea;
        // Foam: each wave's crest running up the sand, and the edge.
        for (int k = 0; k < 3; k++) {
            float f = fract(t * 0.07 + float(k) / 3.0);
            float crest = shore + 0.07 * (1.0 - f) + 0.004 * sin(uv.x * 25.0 + float(k) * 2.0 + t * 0.3);
            float line = smoothstep(0.005, 0.0, abs(uv.y - crest)) * (0.4 + 0.6 * noise(vec2(uv.x * 70.0, float(k) + t * 0.2)));
            line *= smoothstep(0.35, 0.7, noise(vec2(uv.x * 6.0 + float(k) * 11.0, t * 0.05)));
            c += light * 0.3 * line * sin(PI * f) * smoothstep(aspect * 0.68, aspect * 0.5, uv.x);
        }
    }
    // The sand, wet and mirror-like at the water, dry further up.
    float wash = shore + 0.012 * sin(t * 0.45 + uv.x * 2.0);
    if (uv.y < wash) {
        float wet = smoothstep(wash - 0.07, wash, uv.y);
        vec3 sand = mix(c4 * 0.13, low * 0.55, 0.45) * mix(0.45, 1.0, uv.y / wash) * (0.93 + 0.07 * noise(uv * vec2(700.0, 900.0)));
        sand += light * wet * 0.18 * exp(-pow((uv.x - moon.x) / 0.18, 2.0));
        sand = mix(sand, low * 0.5, wet * 0.35);
        c = mix(c, sand, smoothstep(wash, wash - px * 2.0, uv.y));
        c += light * 0.4 * smoothstep(0.005, 0.0, abs(uv.y - wash + 0.002)) * (0.5 + 0.5 * noise(vec2(uv.x * 60.0, t)))
            * smoothstep(aspect * 0.75, aspect * 0.55, uv.x);
    }

    // The headland on the right, a silhouette: a steep face rising from
    // the water to a wooded top, misty low down, the moon along its edge.
    float x0 = aspect * 0.64;
    float base = 0.11 + 0.01 * sin(uv.x * 7.0);
    float plateau = 0.6 + 0.1 * smoothstep(x0, aspect, uv.x) + 0.012 * sin(uv.x * 9.0);
    float crowns = 0.0;
    for (int k = 0; k < 11; k++) {
        float fk = float(k);
        float cx = x0 + 0.01 + fk * 0.05 + 0.02 * hash(vec2(fk, 2.0));
        float r = 0.022 + 0.022 * hash(vec2(fk, 5.0));
        float dx = uv.x - cx;
        crowns = max(crowns, sqrt(max(r * r - dx * dx, 0.0)) - r * 0.3);
    }
    float slope = clamp((uv.x - x0) / 0.32, 0.0, 1.0);
    float ridge = base + (plateau + crowns - base) * smoothstep(0.0, 1.0, pow(slope, 0.6)) * step(x0, uv.x);
    ridge += 0.015 * fbm(vec2(uv.x * 12.0, 3.0)) * (1.0 - slope) * step(x0, uv.x);
    ridge += 0.004 * noise(vec2(uv.x * 160.0, 1.0)) * step(x0 + 0.03, uv.x);
    // The notch the waterfall comes over.
    float fx = aspect * 0.79;
    ridge -= 0.03 * exp(-pow((uv.x - fx) / 0.03, 2.0));
    float land = smoothstep(ridge + px, ridge - px, uv.y) * step(base, uv.y);
    vec3 hill = mix(night * 0.28, mix(low, night, 0.4) * 0.75, smoothstep(plateau, base, uv.y) * 0.6);
    hill += light * 0.18 * exp(-max(ridge - uv.y, 0.0) * 90.0) * smoothstep(-0.02, 0.0, ridge - uv.y);
    c = mix(c, hill, land);

    // The waterfall: over the notch and down the face, widening, the water
    // streaming, brighter at its lip; a glow around it.
    float fall_top = plateau - 0.03 + 0.003;
    float fall_foot = base + 0.01;
    float down = clamp((fall_top - uv.y) / (fall_top - fall_foot), 0.0, 1.0);
    float fw = 0.011 + 0.03 * down;
    float across = (uv.x - fx + 0.01 * down * down) / fw;
    float inside = smoothstep(1.0, 0.5, abs(across)) * step(fall_foot, uv.y) * step(uv.y, fall_top);
    float stream = fbm(vec2(across * 2.5, uv.y * 9.0 + t * 1.6)) * 0.6 + 0.4 * noise(vec2(across * 9.0, uv.y * 30.0 + t * 3.5));
    vec3 water = light * (0.35 + 0.75 * smoothstep(0.3, 0.8, stream)) * (0.85 + 0.35 * smoothstep(0.12, 0.0, down));
    c = mix(c, water, inside * 0.9);
    c += light * 0.06 * exp(-abs(across) * 1.0) * step(fall_foot, uv.y) * step(uv.y, fall_top);
    // The mist rising where it falls, and foam along the headland's foot.
    float mist = fbm(vec2(uv.x * 7.0 - t * 0.06, uv.y * 9.0 - t * 0.35));
    float cloud = exp(-pow((uv.x - fx) / 0.13, 2.0) - pow((uv.y - fall_foot - 0.02) / 0.06, 2.0));
    c += light * 0.4 * cloud * (0.4 + 0.6 * mist);
    float foamline = smoothstep(0.008, 0.0, abs(uv.y - base - 0.003 * sin(uv.x * 40.0 + t)));
    c += light * 0.3 * foamline * step(x0, uv.x) * noise(vec2(uv.x * 50.0, t * 0.7));

    // The palms, and bushes along the bottom on the left.
    float leaning = max(palm(uv, vec2(aspect * 0.04, -0.02), vec2(aspect * 0.16, 0.72), 1.3, px),
                        palm(uv, vec2(aspect * 0.1, -0.02), vec2(aspect * 0.075, 0.55), 4.1, px));
    float bush = 0.06 + 0.09 * fbm(vec2(uv.x * 5.0, 2.0)) * smoothstep(aspect * 0.5, aspect * 0.05, uv.x)
        + 0.012 * noise(vec2(uv.x * 60.0, 3.0)) + 0.005 * noise(vec2(uv.x * 220.0, 4.0));
    float shrubs = smoothstep(bush + px, bush - px, uv.y);
    vec3 dark = night * 0.12;
    c = mix(c, dark + light * 0.03 * smoothstep(bush - 0.03, bush, uv.y), shrubs);
    c = mix(c, dark, leaning);

    // Fireflies about the bushes and under the palms.
    for (int i = 0; i < 32; i++) {
        float fi = float(i);
        vec2 home = vec2(hash(vec2(fi, 1.3)) * aspect * 0.55, 0.04 + 0.32 * pow(hash(vec2(fi, 7.1)), 1.5));
        vec2 drift = 0.035 * vec2(sin(t * (0.25 + 0.2 * hash(vec2(fi, 3.3))) + fi),
                                  cos(t * (0.2 + 0.2 * hash(vec2(fi, 5.5))) + fi * 1.7));
        float d = length(uv - home - drift);
        float blink = pow(max(0.0, sin(t * (0.6 + 0.5 * hash(vec2(fi, 9.9))) + fi * 2.3)), 3.0);
        c += c3 * blink * (exp(-d * d / 0.000012) + 0.3 * exp(-d * 45.0));
    }
    // A vignette.
    vec2 v = v_coords - 0.5;
    c *= 1.0 - 0.45 * dot(v, v);
    finish(clamp(c, 0.0, 1.0));
}
"#;

/// The sky outside, now (`world`): the Sun or the Moon where they are,
/// coloured by how high the Sun is (night, dawn, day, dusk), the stars at
/// night; clouds as many as there are, going with the wind; rain, snow,
/// fog, lightning when there is a storm; hills at the bottom.
///
/// `sky0`: the Sun's altitude and azimuth, then the Moon's (radians; the
/// azimuths from the middle of the screen, rightwards). `sky1`: how much of
/// the Moon is lit, which side (1 the right), the cloud cover, the wind
/// across the screen (-1 … 1). `sky2`: rain, snow, fog, storm, 0..1.
pub const SKY: &str = r#"
// The field of view across the screen, radians.
const float FOV = 1.75;

float rain_layer(vec2 uv, float scale, float speed, float slant) {
    vec2 q = vec2((uv.x + uv.y * slant) * scale, uv.y * scale * 0.08 + time * speed);
    float col = floor(q.x);
    float h = hash(vec2(col, scale));
    float y = fract(q.y + h * 13.0);
    float line = smoothstep(0.08, 0.0, abs(fract(q.x) - 0.5 - (h - 0.5) * 0.6));
    return line * smoothstep(0.75, 0.97, y) * step(0.35, h);
}

float snow_layer(vec2 uv, float scale, float speed, float drift) {
    vec2 q = uv * scale + vec2(time * drift + sin(time * 0.6 + uv.y * 3.0) * 0.3, time * speed);
    vec2 id = floor(q);
    vec2 at = fract(q) - 0.5 - (hash2(id) - 0.5) * 0.7;
    float r = 0.04 + 0.05 * hash(id + 3.1);
    return smoothstep(r, r * 0.3, length(at)) * step(0.45, hash(id + 9.7));
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, 1.0 - v_coords.y);
    float per = aspect / FOV;
    float horizon = 0.24;
    // Where each point of the screen looks: azimuth (from the middle) and
    // elevation, radians.
    vec2 look = vec2((uv.x - aspect * 0.5) / per, (uv.y - horizon) / per);
    float sun_alt = sky0.x;
    vec2 sun = vec2(sky0.y, sky0.x);
    vec2 moon = vec2(sky0.w, sky0.z);
    float lit_moon = sky1.x;
    float cover = sky1.z;
    float wind = sky1.w;
    float rain = sky2.x;
    float snow = sky2.y;
    float fog = sky2.z;
    float storm = sky2.w;

    float day = smoothstep(-0.12, 0.12, sun_alt);
    float dusk = exp(-pow((sun_alt - 0.02) / 0.13, 2.0));
    // How close to the Sun's side of the sky, 1 facing it.
    float towards = exp(-pow(look.x - sun.x, 2.0) * 0.8);

    vec3 zenith = mix(vec3(0.008, 0.012, 0.035), vec3(0.16, 0.38, 0.82), day);
    vec3 low = mix(vec3(0.03, 0.04, 0.08), vec3(0.62, 0.78, 0.95), day);
    low = mix(low, vec3(1.0, 0.48, 0.22), dusk * (0.35 + 0.65 * towards));
    zenith = mix(zenith, vec3(0.2, 0.18, 0.35), dusk * 0.5);
    float e = max(look.y, 0.0);
    vec3 c = mix(low, zenith, smoothstep(0.0, 0.8, e));
    // Grey under a covered sky.
    vec3 grey = mix(vec3(0.03, 0.035, 0.045), vec3(0.55, 0.58, 0.62), day);
    c = mix(c, grey, cover * 0.75);

    // The stars and the Moon, at night.
    float clear = 1.0 - cover;
    c += vec3(stars(uv, 140.0, 0.92) + stars(uv + 4.1, 55.0, 0.985)) * (1.0 - day) * clear * smoothstep(0.0, 0.15, e);
    vec2 dm = (look - moon) * vec2(1.0, 1.0);
    float rm = 0.035;
    float dmoon = length(dm);
    if (moon.y > -0.05) {
        c += vec3(0.75, 0.8, 0.9) * 0.25 * exp(-dmoon * 9.0) * lit_moon * (1.0 - 0.7 * day) * (1.0 - 0.6 * cover);
        if (dmoon < rm) {
            vec2 m = dm / rm;
            float x = m.x * sky1.y;
            float edge = (1.0 - 2.0 * lit_moon) * sqrt(max(1.0 - m.y * m.y, 0.0));
            float shine = smoothstep(edge - 0.06, edge + 0.06, x);
            vec3 face = vec3(0.86, 0.87, 0.9) * (0.82 + 0.25 * fbm(m * 3.0 + 7.0));
            vec3 moon_c = mix(c * 0.92 + 0.012, face, shine);
            c = mix(c, moon_c, smoothstep(rm, rm * 0.94, dmoon) * (1.0 - 0.8 * cover));
        }
    }
    // The Sun, and its glow.
    float ds = length(look - sun);
    vec3 sun_c = mix(vec3(1.0, 0.55, 0.25), vec3(1.0, 0.97, 0.9), smoothstep(0.0, 0.3, sun_alt));
    float veil = 1.0 - 0.85 * cover;
    c += sun_c * (0.5 * exp(-ds * 5.0) + 0.25 * exp(-ds * 1.5)) * smoothstep(-0.15, 0.0, sun_alt) * veil;
    c = mix(c, sun_c * 1.2, smoothstep(0.03, 0.025, ds) * veil * smoothstep(-0.03, 0.0, sun_alt));

    // Clouds on a plane above: nearer the horizon, further and flatter.
    if (look.y > -0.02) {
        float far = 1.0 / (max(look.y, 0.0) + 0.12);
        vec2 plane = vec2(look.x * far * 0.9, far) * 0.75;
        float t = time * (0.012 + 0.05 * abs(wind));
        vec2 flow = vec2(wind >= 0.0 ? 1.0 : -1.0, 0.35);
        float body = fbm(plane + flow * t * 3.0);
        body = 0.65 * body + 0.35 * fbm(plane * 2.3 - flow * t * 4.5 + 5.0);
        float edge = mix(0.68, 0.18, cover);
        float amount = smoothstep(edge, edge + 0.22, body);
        amount *= smoothstep(-0.02, 0.06, look.y);
        float thick = smoothstep(0.4, 0.95, body);
        vec3 cloud_day = mix(vec3(0.97, 0.97, 0.98), vec3(0.5, 0.53, 0.58), thick * (0.4 + 0.6 * cover));
        cloud_day = mix(cloud_day, vec3(0.6, 0.62, 0.66) * (0.85 + 0.3 * body), cover * cover * 0.8);
        vec3 cloud_dusk = mix(vec3(1.0, 0.6, 0.4), vec3(0.35, 0.25, 0.35), thick);
        vec3 cloud_night = vec3(0.04, 0.045, 0.06) + vec3(0.12, 0.13, 0.15) * lit_moon * (1.0 - thick) * exp(-length(look - moon) * 1.5);
        vec3 cloud = mix(cloud_night, cloud_day, day);
        cloud = mix(cloud, cloud_dusk, dusk * (0.3 + 0.7 * towards) * (1.0 - cover * 0.6));
        cloud *= 1.0 - 0.45 * (rain + storm * 0.5);
        c = mix(c, cloud, amount);
    }

    // Lightning: the sky flashes now and then.
    float beat = floor(time * 3.0);
    float flash = storm * step(0.975, hash(vec2(beat, 1.7))) * exp(-fract(time * 3.0) * 5.0);
    c += vec3(0.7, 0.75, 0.9) * flash;

    // The hills: a far range pale with the distance, nearer ones rolling,
    // trees along their tops.
    float far_hill = horizon + 0.015 + 0.09 * fbm(vec2(uv.x * 1.3, 3.0));
    float near_hill = horizon - 0.07 + 0.11 * fbm(vec2(uv.x * 1.9 + 9.0, 1.0));
    near_hill += 0.018 * pow(noise(vec2(uv.x * 38.0, 2.0)), 2.0) + 0.006 * noise(vec2(uv.x * 140.0, 4.0));
    vec3 land_day = vec3(0.13, 0.22, 0.12);
    vec3 night_land = vec3(0.012, 0.014, 0.024);
    vec3 far_c = mix(mix(night_land * 2.5, mix(land_day, low, 0.55), day), c, 0.35 + fog * 0.5);
    float shade = smoothstep(near_hill, near_hill - 0.25, uv.y);
    vec3 near_c = mix(night_land, land_day * (1.0 - 0.45 * cover) * (1.0 - 0.4 * shade), day);
    near_c = mix(near_c, c, fog * 0.35) + vec3(flash) * 0.08;
    // The light of the low Sun along the ridges.
    near_c += sun_c * 0.25 * dusk * towards * exp(-(near_hill - uv.y) * 60.0) * veil;
    c = mix(c, far_c, smoothstep(far_hill + 0.002, far_hill - 0.002, uv.y));
    c = mix(c, near_c, smoothstep(near_hill + 0.002, near_hill - 0.002, uv.y));
    // Snow lies on the ground.
    c = mix(c, mix(vec3(0.05, 0.055, 0.07), vec3(0.85, 0.87, 0.9), day) * (1.0 - 0.3 * shade), snow * 0.8 * step(uv.y, near_hill));

    // Fog over everything, thickest low down.
    vec3 mist = mix(vec3(0.05, 0.055, 0.06), vec3(0.72, 0.74, 0.76), day);
    c = mix(c, mist, fog * (0.45 + 0.4 * smoothstep(0.6, 0.0, uv.y)));
    // Rain and snow in front, slanting with the wind.
    float slant = -wind * 0.25;
    float drops = rain_layer(uv, 90.0, 9.0, slant) + 0.6 * rain_layer(uv + 0.37, 150.0, 6.0, slant);
    c = mix(c, mix(vec3(0.35, 0.38, 0.45), vec3(0.85, 0.88, 0.92), day), clamp(drops * (0.4 + rain), 0.0, 1.0) * 0.7 * step(0.01, rain));
    c *= 1.0 - 0.18 * rain;
    float flakes = snow_layer(uv, 14.0, 0.25, wind * 0.4) + 0.7 * snow_layer(uv + 0.5, 24.0, 0.15, wind * 0.3)
        + 0.5 * snow_layer(uv + 0.2, 40.0, 0.1, wind * 0.2);
    c = mix(c, vec3(0.92, 0.94, 0.97) * (0.35 + 0.65 * day), clamp(flakes * snow, 0.0, 1.0));
    finish(clamp(c, 0.0, 1.0));
}
"#;
