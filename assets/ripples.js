// Ripples: a ripple tank on the GPU. A 2D wave equation with reflecting walls, drawn as stippled
// dots, so wavefronts read as thin shimmering rings and crossings interfere.
//
//   const r = Ripples(canvas, { ambient: true });
//   r.drop(x, y, strength, radius)   // x, y in 0..1 of the canvas; strength ~0.2..2
//   r.poke(x, y, click)              // a cursor: small drops as it moves, a big one on click
//   r.drive(level, x, y)             // audio: level 0..1 each frame; onsets become drops
//   r.pause(true | false); r.colors([r,g,b], [r,g,b]); r.destroy()
(function () {
  const VS = `#version 300 es
in vec2 p; out vec2 uv;
void main() { uv = p * 0.5 + 0.5; gl_Position = vec4(p, 0.0, 1.0); }`;

  // r = height now, g = height a step ago. Clamped edges reflect.
  const SIM = `#version 300 es
precision highp float;
uniform sampler2D s; uniform vec2 texel; uniform float damp;
uniform vec4 drops[16]; uniform int n;
in vec2 uv; out vec4 o;
void main() {
  vec2 c = texture(s, uv).rg;
  float l = texture(s, uv + vec2(texel.x, 0.0)).r + texture(s, uv - vec2(texel.x, 0.0)).r
          + texture(s, uv + vec2(0.0, texel.y)).r + texture(s, uv - vec2(0.0, texel.y)).r - 4.0 * c.r;
  float h = (2.0 * c.r - c.g + 0.48 * l) * damp;
  for (int i = 0; i < 16; i++) {
    if (i >= n) break;
    // A packet of concentric ripples with no net volume: it leaves as a ring train, not a
    // bump that raises the whole tank.
    float d = length((uv - drops[i].xy) / texel), r = drops[i].z;
    h += drops[i].w * cos(d / r * 2.0) * exp(-d * d / (r * r * 30.0));
  }
  o = vec4(h, c.r, 0.0, 1.0);
}`;

  // Crests become dots: the brighter the crest, the denser the stipple. The grain reshuffles
  // every few frames, so rings shimmer the way sand on a speaker does.
  const DRAW = `#version 300 es
precision highp float;
uniform sampler2D s; uniform float seed, gain; uniform vec3 ink, paper;
in vec2 uv; out vec4 o;
float hash(vec2 p) { vec3 q = fract(vec3(p.xyx) * 0.1031); q += dot(q, q.yzx + 33.33); return fract((q.x + q.y) * q.z); }
void main() {
  float h = texture(s, uv).r * gain;
  // Crests never go solid: at most about half the grains light, like sand on a plate.
  float b = 0.58 * smoothstep(0.06, 1.0, h) + 0.02 * clamp(abs(h), 0.0, 1.0);
  float lit = step(hash(gl_FragCoord.xy + seed), b);
  o = vec4(mix(paper, ink, lit), 1.0);
}`;

  const COPY = `#version 300 es
precision highp float;
uniform sampler2D s; in vec2 uv; out vec4 o;
void main() { o = texture(s, uv); }`;

  function compile(gl, vs, fs) {
    const prog = gl.createProgram();
    for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
      const sh = gl.createShader(type);
      gl.shaderSource(sh, src); gl.compileShader(sh);
      if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh));
      gl.attachShader(prog, sh);
    }
    gl.bindAttribLocation(prog, 0, 'p');
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
    const u = {};
    const count = gl.getProgramParameter(prog, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < count; i++) { const name = gl.getActiveUniform(prog, i).name.replace('[0]', ''); u[name] = gl.getUniformLocation(prog, name); }
    return { prog, u };
  }

  window.Ripples = function (canvas, opts = {}) {
    const o = Object.assign({ scale: 0.5, steps: 2, damp: 0.9965, fps: 60, ambient: true, gain: 9, ink: [1, 1, 1], paper: [0, 0, 0] }, opts);
    const gl = canvas.getContext('webgl2', { antialias: false, alpha: false, premultipliedAlpha: false, powerPreference: 'low-power' });
    if (!gl) return null;
    const half = !gl.getExtension('EXT_color_buffer_float');
    if (half && !gl.getExtension('EXT_color_buffer_half_float')) return null;
    const sim = compile(gl, VS, SIM), draw = compile(gl, VS, DRAW), copy = compile(gl, VS, COPY);
    const quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    let W = 0, H = 0, tex = [], fbo = [], cur = 0, pending = [], paused = false, raf = 0, last = 0, frame = 0;
    // New water at the new size, carrying the old water over, so a resize doesn't wipe it.
    const make = () => {
      const old = tex[cur], oldTex = tex, oldFbo = fbo;
      tex = []; fbo = [];
      for (let i = 0; i < 2; i++) {
        const t = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, half ? gl.RG16F : gl.RG32F, W, H, 0, gl.RG, half ? gl.HALF_FLOAT : gl.FLOAT, null);
        const filter = half ? gl.LINEAR : (gl.getExtension('OES_texture_float_linear') ? gl.LINEAR : gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        const f = gl.createFramebuffer();
        gl.bindFramebuffer(gl.FRAMEBUFFER, f);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
        gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
        tex.push(t); fbo.push(f);
        if (old) {
          gl.useProgram(copy.prog); gl.viewport(0, 0, W, H);
          gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, old); gl.uniform1i(copy.u.s, 0);
          gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        }
      }
      oldTex.forEach(t => gl.deleteTexture(t)); oldFbo.forEach(f => gl.deleteFramebuffer(f));
      cur = 0;
    };
    const resize = () => {
      // One canvas pixel per CSS pixel: dots land two device pixels wide on Retina, like grains.
      const cw = Math.max(1, Math.round(canvas.clientWidth)), ch = Math.max(1, Math.round(canvas.clientHeight));
      if (canvas.width !== cw || canvas.height !== ch) { canvas.width = cw; canvas.height = ch; }
      const w = Math.max(16, Math.round(cw * o.scale)), h = Math.max(16, Math.round(ch * o.scale));
      if (w !== W || h !== H) { W = w; H = h; make(); }
    };

    const api = {
      drop(x, y, strength = 1, radius = 2.2) { pending.push([x, 1 - y, radius, strength * 0.9]); if (pending.length > 16) pending.shift(); },
      poke(x, y, click) {
        const t = performance.now();
        if (click) { api.drop(x, y, 1.6, 3); return; }
        if (t - (api._lp || 0) > 45) { api._lp = t; api.drop(x, y, 0.18, 1.6); }
      },
      // Audio: a rising level is an onset, and an onset drops a ring. Loud = bigger.
      drive(level, x = 0.5, y = 0.5) {
        const avg = api._avg = (api._avg ?? level) * 0.92 + level * 0.08;
        const t = performance.now();
        if (level > 0.02 && level > avg * 1.35 && t - (api._lo || 0) > 70) { api._lo = t; api.drop(x, y, Math.min(2.2, 0.4 + level * 3), 1.8 + level * 2.5); }
        else if (level > 0.03 && t - (api._lc || 0) > 120) { api._lc = t; api.drop(x + (Math.random() - 0.5) * 0.01, y + (Math.random() - 0.5) * 0.01, level * 0.8, 1.6); }
      },
      pause(p) { paused = !!p; if (!paused && !raf) raf = requestAnimationFrame(tick); },
      colors(ink, paper) { o.ink = ink; o.paper = paper; },
      set(k, v) { o[k] = v; },
      destroy() { cancelAnimationFrame(raf); raf = 0; paused = true; },
      // Runs the water ahead without drawing, so a page opens on rings already travelling.
      advance(n) { resize(); step(n); paint(); },
    };

    let nextAmbient = 0;
    function ambient(t) {
      if (!o.ambient || t < nextAmbient) return;
      const x = 0.08 + Math.random() * 0.84, y = 0.08 + Math.random() * 0.84;
      api.drop(x, y, 0.7 + Math.random() * 1.1, 2 + Math.random() * 2);
      // Now and then a second source close by, so the rings cross and interfere.
      if (Math.random() < 0.35) api.drop(x + (Math.random() - 0.5) * 0.2, y + (Math.random() - 0.5) * 0.2, 0.6 + Math.random() * 0.8, 2 + Math.random());
      nextAmbient = t + 1400 + Math.random() * 3200;
    }

    function tick(t) {
      raf = 0;
      if (paused) return;
      raf = requestAnimationFrame(tick);
      if (t - last < 1000 / o.fps - 1) return;
      last = t; frame++;
      resize(); ambient(t);
      step(o.steps);
      paint();
    }
    function step(count) {
      gl.useProgram(sim.prog);
      gl.uniform2f(sim.u.texel, 1 / W, 1 / H);
      gl.uniform1f(sim.u.damp, o.damp);
      gl.viewport(0, 0, W, H);
      for (let i = 0; i < count; i++) {
        const ds = i === 0 ? pending.splice(0, 16) : [];
        const flat = new Float32Array(64);
        ds.forEach((d, j) => flat.set(d, j * 4));
        gl.uniform4fv(sim.u.drops, flat);
        gl.uniform1i(sim.u.n, ds.length);
        gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, tex[cur]);
        gl.uniform1i(sim.u.s, 0);
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbo[1 - cur]);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        cur = 1 - cur;
      }
    }
    function paint() {
      gl.useProgram(draw.prog);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.bindTexture(gl.TEXTURE_2D, tex[cur]);
      gl.uniform1i(draw.u.s, 0);
      gl.uniform1f(draw.u.seed, Math.floor(frame / 2) * 17.31 % 1000);
      gl.uniform1f(draw.u.gain, o.gain);
      gl.uniform3fv(draw.u.ink, o.ink); gl.uniform3fv(draw.u.paper, o.paper);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    resize();
    if (o.warm) { for (let i = 0; i < 3; i++) ambient(Infinity), nextAmbient = 0; step(o.warm); paint(); }
    raf = requestAnimationFrame(tick);
    return api;
  };
})();
