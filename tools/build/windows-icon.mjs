/** Tauri Windows codegen uses ICO entry zero as the native window icon. */
export const WINDOWS_ICON_SIZES = [256, 192, 128, 96, 80, 64, 48, 40, 32, 24, 20, 16];
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

export function validateWindowsIcon(bytes, desktopPng) {
  if (bytes.length < 6 || bytes.readUInt16LE(0) !== 0 || bytes.readUInt16LE(2) !== 1)
    throw Error('Invalid Windows ICO header');
  const count = bytes.readUInt16LE(4);
  if (count !== WINDOWS_ICON_SIZES.length || bytes.length < 6 + count * 16)
    throw Error('Windows ICO is missing DPI frames');
  let nextOffset = 6 + count * 16;
  for (const [index, size] of WINDOWS_ICON_SIZES.entries()) {
    const entry = 6 + index * 16;
    const width = bytes[entry] || 256;
    const height = bytes[entry + 1] || 256;
    if (width !== size || height !== size)
      throw Error('Windows ICO must start with 256px and retain descending DPI frames');
    const length = bytes.readUInt32LE(entry + 8);
    const offset = bytes.readUInt32LE(entry + 12);
    if (offset !== nextOffset || length < 33 || offset + length > bytes.length)
      throw Error('Invalid Windows ICO frame bounds');
    const png = bytes.subarray(offset, offset + length);
    if (!png.subarray(0, 8).equals(pngSignature) || png.toString('ascii', 12, 16) !== 'IHDR'
        || png.readUInt32BE(16) !== size || png.readUInt32BE(20) !== size
        || png[24] !== 8 || png[25] !== 6 || bytes.readUInt16LE(entry + 6) !== 32)
      throw Error('Windows ICO frames must be correctly sized 32-bit RGBA PNGs');
    if (index === 0 && !png.equals(desktopPng))
      throw Error('Windows window and installer icons differ');
    nextOffset += length;
  }
  if (nextOffset !== bytes.length) throw Error('Unexpected data after Windows ICO frames');
}
