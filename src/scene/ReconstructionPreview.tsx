/** Read-only views of a validated reconstruction preview volume. */
import { useEffect, useRef, useState } from "react";

export interface PreviewVolume {
  shape: [number, number, number]; // Z, Y, X
  spacingMm: [number, number, number];
  windowMin: number;
  windowMax: number;
  voxels: number[];
}

type Axis = "z" | "y" | "x";
type Position = { z: number; y: number; x: number };

function voxel(volume: PreviewVolume, z: number, y: number, x: number): number {
  return volume.voxels[(z * volume.shape[1] + y) * volume.shape[2] + x] ?? 0;
}

function Slice({ volume, axis, position, onPosition }: {
  volume: PreviewVolume;
  axis: Axis;
  position: Position;
  onPosition: (axis: Axis, value: number) => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [zSize, ySize, xSize] = volume.shape;
  const width = axis === "x" ? ySize : xSize;
  const height = axis === "z" ? ySize : zSize;
  const index = position[axis];
  const count = axis === "z" ? zSize : axis === "y" ? ySize : xSize;
  useEffect(() => {
    const context = canvas.current?.getContext("2d", { alpha: false });
    if (!context) return;
    const pixels = context.createImageData(width, height);
    for (let row = 0; row < height; row += 1) {
      for (let column = 0; column < width; column += 1) {
        const value = axis === "z"
          ? voxel(volume, index, ySize - 1 - row, column)
          : axis === "y"
            ? voxel(volume, zSize - 1 - row, index, column)
            : voxel(volume, zSize - 1 - row, ySize - 1 - column, index);
        const intensity = Math.round(Math.max(0, Math.min(255, value)));
        const offset = (row * width + column) * 4;
        pixels.data[offset] = intensity;
        pixels.data[offset + 1] = intensity;
        pixels.data[offset + 2] = intensity;
        pixels.data[offset + 3] = 255;
      }
    }
    context.putImageData(pixels, 0, 0);
    const crossX = axis === "x" ? ySize - 1 - position.y : position.x;
    const crossY = axis === "z" ? ySize - 1 - position.y : zSize - 1 - position.z;
    context.strokeStyle = "rgba(69, 182, 204, .65)";
    context.lineWidth = 1;
    context.beginPath();
    context.moveTo(crossX + .5, 0); context.lineTo(crossX + .5, height);
    context.moveTo(0, crossY + .5); context.lineTo(width, crossY + .5);
    context.stroke();
  }, [axis, height, index, position, volume, width, xSize, ySize, zSize]);

  const title = axis === "z" ? "XY · AXIAL" : axis === "y" ? "XZ · CORONAL" : "YZ · SAGITTAL";
  const marker = axis.toUpperCase();
  const spacing = volume.spacingMm[axis === "z" ? 0 : axis === "y" ? 1 : 2];
  return <section className="recon-pane recon-pane--slice" aria-label={`${title} slice`}>
    <div className="recon-pane__head"><strong>{title}</strong><span>{marker} {((index - (count - 1) / 2) * spacing).toFixed(2)} mm</span></div>
    <div className="recon-slice-stage" onPointerDown={(event) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const displayScale = Math.min(rect.width / width, rect.height / height);
      const displayWidth = width * displayScale;
      const displayHeight = height * displayScale;
      const u = Math.max(0, Math.min(0.999, (event.clientX - rect.left - (rect.width - displayWidth) / 2) / displayWidth));
      const v = Math.max(0, Math.min(0.999, (event.clientY - rect.top - (rect.height - displayHeight) / 2) / displayHeight));
      if (axis !== "x") onPosition("x", Math.floor(u * xSize));
      if (axis === "z") onPosition("y", ySize - 1 - Math.floor(v * ySize));
      if (axis === "y") onPosition("z", zSize - 1 - Math.floor(v * zSize));
      if (axis === "x") { onPosition("y", ySize - 1 - Math.floor(u * ySize)); onPosition("z", zSize - 1 - Math.floor(v * zSize)); }
    }}>
      <canvas ref={canvas} width={width} height={height} aria-label={`${title} reconstructed data`} />
      <span className="recon-axis recon-axis--top">+{axis === "z" ? "Y" : "Z"}</span>
      <span className="recon-axis recon-axis--right">+{axis === "x" ? "Y" : "X"}</span>
      <span className="recon-axis recon-axis--bottom">−{axis === "z" ? "Y" : "Z"}</span>
      <span className="recon-axis recon-axis--left">−{axis === "x" ? "Y" : "X"}</span>
    </div>
    <input type="range" min={0} max={Math.max(0, count - 1)} value={index} aria-label={`${marker} slice position`} onChange={(event) => onPosition(axis, Number(event.target.value))} />
  </section>;
}

const vertexShader = `#version 300 es
void main(){vec2 p=vec2(gl_VertexID==1?3.0:-1.0,gl_VertexID==2?3.0:-1.0);gl_Position=vec4(p,0.,1.);}`;
// The shader is built once per canvas; the full fragment source is kept apart
// from controls so the scene never observes or changes acquisition state.
const volumeFragment = `#version 300 es
precision highp float;precision highp sampler3D;out vec4 color;
uniform sampler3D volume;uniform float yaw,pitch,span,aspect;uniform vec2 viewport;uniform int mode;
void main(){
  vec3 look=normalize(vec3(sin(yaw)*cos(pitch),-cos(yaw)*cos(pitch),sin(pitch)));
  vec3 right=normalize(vec3(cos(yaw),sin(yaw),0.));vec3 up=normalize(cross(look,right));
  vec2 q=(gl_FragCoord.xy/viewport-.5)*2.;
  vec3 origin=look*1.7+(right*q.x*aspect+up*q.y)*span*.5;vec3 ray=-look;
  vec3 safe=sign(ray+vec3(1e-8))*max(abs(ray),vec3(1e-6));
  vec3 first=(-vec3(.5)-origin)/safe,last=(vec3(.5)-origin)/safe;
  vec3 nearSide=min(first,last),farSide=max(first,last);
  float t=max(0.,max(max(nearSide.x,nearSide.y),nearSide.z));
  float stop=min(min(farSide.x,farSide.y),farSide.z);
  vec3 background=vec3(.025,.055,.087);
  if(stop<t){color=vec4(background,1.);return;}
  float peak=0.,opacity=0.;vec3 accum=vec3(0.);
  for(int i=0;i<340;i++){
    if(t>stop||opacity>.985)break;
    float value=texture(volume,origin+ray*t+vec3(.5)).r;
    peak=max(peak,value);
    if(mode==1){float a=smoothstep(.16,.70,value)*.048;vec3 tone=vec3(value*.73,value*.89,value);accum+=(1.-opacity)*a*tone;opacity+=(1.-opacity)*a;}
    t+=1./184.;
  }
  color=mode==0?vec4(background*(1.-smoothstep(.03,.40,peak))+vec3(peak),1.):vec4(accum+background*(1.-opacity),1.);
}`;

function VolumeView({ volume, mode, resetRevision, onVolumeAvailable }: { volume: PreviewVolume; mode: "mip" | "opacity"; resetRevision: number; onVolumeAvailable: (available: boolean) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const renderer = useRef<(() => void) | null>(null);
  const pose = useRef({ yaw: 0.55, pitch: 0.35, span: 1.15, mode: 0, x: 0, y: 0, dragging: false });
  const [error, setError] = useState<string | null>(null);
  const [angle, setAngle] = useState("32° / 20°");
  useEffect(() => {
    pose.current.mode = mode === "mip" ? 0 : 1;
    renderer.current?.();
  }, [mode]);
  useEffect(() => {
    pose.current.yaw = .55;
    pose.current.pitch = .35;
    pose.current.span = 1.15;
    renderer.current?.();
  }, [resetRevision]);
  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    onVolumeAvailable(false);
    const gl = element.getContext("webgl2", { alpha: false, antialias: false, depth: false });
    if (!gl) {
      const context = element.getContext("2d", { alpha: false });
      if (context) {
        const [z, y, x] = volume.shape;
        element.width = x; element.height = y;
        const pixels = context.createImageData(x, y);
        for (let row = 0; row < y; row += 1) for (let column = 0; column < x; column += 1) {
          let peak = 0;
          for (let slice = 0; slice < z; slice += 1) peak = Math.max(peak, voxel(volume, slice, y - 1 - row, column));
          const offset = (row * x + column) * 4;
          pixels.data[offset] = peak; pixels.data[offset + 1] = peak; pixels.data[offset + 2] = peak; pixels.data[offset + 3] = 255;
        }
        context.putImageData(pixels, 0, 0);
      }
      setError("WebGL2 unavailable · static MIP");
      return;
    }
    let program: WebGLProgram | null = null;
    let texture: WebGLTexture | null = null;
    let frame = 0;
    const shader = (kind: number, source: string): WebGLShader => {
      const result = gl.createShader(kind);
      if (!result) throw new Error("Shader creation failed");
      gl.shaderSource(result, source); gl.compileShader(result);
      if (!gl.getShaderParameter(result, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(result) || "Shader compilation failed");
      return result;
    };
    try {
      program = gl.createProgram();
      if (!program) throw new Error("Render program creation failed");
      const vs = shader(gl.VERTEX_SHADER, vertexShader);
      const fs = shader(gl.FRAGMENT_SHADER, volumeFragment);
      gl.attachShader(program, vs); gl.attachShader(program, fs); gl.linkProgram(program);
      gl.deleteShader(vs); gl.deleteShader(fs);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) || "Render program linking failed");
      const [z, y, x] = volume.shape;
      const bytes = Uint8Array.from(volume.voxels);
      texture = gl.createTexture();
      if (!texture) throw new Error("Volume texture creation failed");
      gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_3D, texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      for (const wrap of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T, gl.TEXTURE_WRAP_R]) gl.texParameteri(gl.TEXTURE_3D, wrap, gl.CLAMP_TO_EDGE);
      gl.texImage3D(gl.TEXTURE_3D, 0, gl.R8, x, y, z, 0, gl.RED, gl.UNSIGNED_BYTE, bytes);
      if (gl.getError() !== gl.NO_ERROR) throw new Error("Volume texture upload failed");
      gl.useProgram(program);
      gl.uniform1i(gl.getUniformLocation(program, "volume"), 0);
      const activeProgram = program;
      const draw = () => {
        frame = 0;
        const width = Math.max(1, Math.round(element.clientWidth));
        const height = Math.max(1, Math.round(element.clientHeight));
        if (element.width !== width || element.height !== height) { element.width = width; element.height = height; }
        gl.viewport(0, 0, width, height);
        gl.useProgram(activeProgram);
        gl.uniform1f(gl.getUniformLocation(activeProgram, "yaw"), pose.current.yaw);
        gl.uniform1f(gl.getUniformLocation(activeProgram, "pitch"), pose.current.pitch);
        gl.uniform1f(gl.getUniformLocation(activeProgram, "span"), pose.current.span);
        gl.uniform1f(gl.getUniformLocation(activeProgram, "aspect"), width / height);
        gl.uniform2f(gl.getUniformLocation(activeProgram, "viewport"), width, height);
        gl.uniform1i(gl.getUniformLocation(activeProgram, "mode"), pose.current.mode);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        setAngle(`${Math.round(pose.current.yaw * 180 / Math.PI)}° / ${Math.round(pose.current.pitch * 180 / Math.PI)}°`);
      };
      renderer.current = () => { if (!frame) frame = requestAnimationFrame(draw); };
      const observer = new ResizeObserver(() => renderer.current?.());
      const contextLost = (event: Event) => {
        event.preventDefault();
        onVolumeAvailable(false);
        setError("WebGL2 context lost. Reopen the preview to retry.");
      };
      element.addEventListener("webglcontextlost", contextLost);
      observer.observe(element);
      renderer.current();
      onVolumeAvailable(true);
      return () => {
        element.removeEventListener("webglcontextlost", contextLost);
        observer.disconnect(); cancelAnimationFrame(frame); renderer.current = null;
        if (texture) gl.deleteTexture(texture);
        if (program) gl.deleteProgram(program);
        onVolumeAvailable(false);
      };
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      if (texture) gl.deleteTexture(texture);
      if (program) gl.deleteProgram(program);
    }
  }, [volume, onVolumeAvailable]);
  return <section className="recon-pane recon-pane--volume" aria-label="Interactive 3D reconstruction">
    <div className="recon-pane__head"><strong>3D · {mode === "mip" ? "MIP" : "Opacity"}</strong><span>{angle}</span></div>
    <div className="recon-volume-stage">
      <canvas ref={canvas} aria-label="Drag to rotate reconstructed volume" onPointerDown={(event) => { pose.current.dragging = true; pose.current.x = event.clientX; pose.current.y = event.clientY; event.currentTarget.setPointerCapture(event.pointerId); }} onPointerMove={(event) => {
        if (!pose.current.dragging) return;
        pose.current.yaw += (event.clientX - pose.current.x) * .008;
        pose.current.pitch = Math.max(-1.48, Math.min(1.48, pose.current.pitch + (event.clientY - pose.current.y) * .008));
        pose.current.x = event.clientX; pose.current.y = event.clientY; renderer.current?.();
      }} onPointerUp={() => { pose.current.dragging = false; }} onPointerCancel={() => { pose.current.dragging = false; }} onWheel={(event) => { event.preventDefault(); pose.current.span = Math.max(.55, Math.min(2.7, pose.current.span * Math.exp(event.deltaY * .001))); renderer.current?.(); }} onDoubleClick={() => { pose.current.yaw = .55; pose.current.pitch = .35; pose.current.span = 1.15; renderer.current?.(); }} />
      {error && <div className="recon-volume-error" role="status">{error}</div>}
    </div>
    <div className="recon-volume-tools"><span title="Entire reconstructed volume is shown; no coverage-mask clipping. Low-signal, low-gain and partially covered regions may contain artifacts and are not quantitative. Signed values and coverage diagnostics are saved separately.">WINDOW {volume.windowMin.toFixed(2)}–{volume.windowMax.toFixed(2)} · NOT HU · FULL VOLUME</span></div>
  </section>;
}

export function ReconstructionPreview({ volume, mode, resetRevision, onVolumeAvailable }: { volume: PreviewVolume; mode: "mip" | "opacity"; resetRevision: number; onVolumeAvailable: (available: boolean) => void }) {
  const [position, setPosition] = useState<Position>(() => ({ z: Math.floor(volume.shape[0] / 2), y: Math.floor(volume.shape[1] / 2), x: Math.floor(volume.shape[2] / 2) }));
  const onPosition = (axis: Axis, value: number) => setPosition(current => ({ ...current, [axis]: value }));
  return <div className="recon-grid">
    <Slice volume={volume} axis="z" position={position} onPosition={onPosition} />
    <Slice volume={volume} axis="y" position={position} onPosition={onPosition} />
    <Slice volume={volume} axis="x" position={position} onPosition={onPosition} />
    <VolumeView volume={volume} mode={mode} resetRevision={resetRevision} onVolumeAvailable={onVolumeAvailable} />
  </div>;
}
