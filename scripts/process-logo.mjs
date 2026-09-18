import sharp from "sharp";

// Asset-only transformation: remove neutral white pixels from the supplied logo
// and add consistent icon-safe padding without changing the brand geometry.

const source = "E:/Main/OneDrive/LanZhouUniv/Class/mluti-energy imaging/CT/前端/logo.png";
const output = "E:/Main/OneDrive/LanZhouUniv/Class/mluti-energy imaging/CT/Micro-CT-App/public/assets/micro-ct-logo.png";

const image = sharp(source).ensureAlpha();
const { data, info } = await image.raw().toBuffer({ resolveWithObject: true });
for (let index = 0; index < info.width * info.height; index += 1) {
  const offset = index * info.channels;
  const red = data[offset];
  const green = data[offset + 1];
  const blue = data[offset + 2];
  const minimum = Math.min(red, green, blue);
  const maximum = Math.max(red, green, blue);
  const chroma = maximum - minimum;
  if (minimum > 205 && chroma < 20) {
    data[offset + 3] = 0;
  } else if (minimum > 205) {
    data[offset + 3] = Math.max(0, Math.min(255, chroma * 12));
  } else {
    data[offset + 3] = 255;
  }
}
const processed = await sharp(data, {
  raw: { width: info.width, height: info.height, channels: info.channels },
})
  .resize({ width: 860, height: 860, fit: "inside" })
  .png()
  .toBuffer();
await sharp({
  create: { width: 1024, height: 1024, channels: 4, background: { r: 0, g: 0, b: 0, alpha: 0 } },
})
  .composite([{ input: processed, gravity: "center" }])
  .png()
  .toFile(output);
