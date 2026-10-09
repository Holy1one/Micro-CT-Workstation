"""Maintain the two MSI bitmap resources from the existing transparent logo.

Pillow is only needed when maintaining these assets. The three-times standard
WiX dimensions supply sharp pixels at this workstation's 250% Windows scaling.
"""
from pathlib import Path
from PIL import Image

assets = Path(__file__).resolve().parents[2] / 'assets'
source = Image.open(assets / 'micro-ct-logo.png').convert('RGBA')
source = source.crop(source.getchannel('A').getbbox())


def logo(size):
    factor = size / max(source.size)
    dimensions = tuple(round(length * factor) for length in source.size)
    return source.convert('RGBa').resize(dimensions, Image.Resampling.LANCZOS).convert('RGBA')


banner = Image.new('RGBA', (1479, 174), 'white')
mark = logo(138)
banner.alpha_composite(mark, (1479 - 160, (174 - mark.height) // 2))
banner.convert('RGB').save(assets / 'installer-banner.bmp')
dialog = Image.new('RGBA', (1479, 936), 'white')
dialog.paste((240, 246, 252), (0, 0, 490, 936))
mark = logo(330)
dialog.alpha_composite(mark, ((490 - mark.width) // 2, (936 - mark.height) // 2))
dialog.convert('RGB').save(assets / 'installer-dialog.bmp')
print('PASS installer banner/dialog bitmaps maintained at 3x WiX dimensions')
