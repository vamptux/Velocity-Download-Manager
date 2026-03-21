import { Jimp } from 'jimp';

async function processImage() {
  // Read the original image
  const image = await Jimp.read('src/veloicon.png');
  
  // Autocrop the transparent borders
  image.autocrop();
  
  // Now we have the tight bounds of the actual logo.
  const width = image.bitmap.width;
  const height = image.bitmap.height;
  const size = Math.max(width, height);
  
  // Create a new square image with a small 2% padding
  const padding = Math.floor(size * 0.02);
  const fullSize = size + padding * 2;
  
  const square = new Jimp({ width: fullSize, height: fullSize, color: 0x00000000 });
  
  // Center the cropped image onto the square
  const x = Math.floor((fullSize - width) / 2);
  const y = Math.floor((fullSize - height) / 2);
  
  square.composite(image, x, y);
  
  await square.write('cropped_square_icon.png');
  console.log('Done!');
}

processImage().catch(console.error);