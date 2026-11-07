// Synthetic archive format fixtures, never signed native artifacts or production trust.
export function archiveFixture(name = "fixture.txt", text = "fixture", mode = 0): Buffer {
  const filename = Buffer.from(name), bytes = Buffer.from(text);
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  crc = (crc ^ 0xffffffff) >>> 0;
  const local = Buffer.alloc(30), central = Buffer.alloc(46), end = Buffer.alloc(22);
  local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4);
  local.writeUInt32LE(crc, 14); local.writeUInt32LE(bytes.length, 18);
  local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(filename.length, 26);
  central.writeUInt32LE(0x02014b50); central.writeUInt16LE(0x0314, 4); central.writeUInt16LE(20, 6);
  central.writeUInt32LE(crc, 16); central.writeUInt32LE(bytes.length, 20);
  central.writeUInt32LE(bytes.length, 24); central.writeUInt16LE(filename.length, 28);
  central.writeUInt32LE((mode << 16) >>> 0, 38);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(1, 8); end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + filename.length, 12);
  end.writeUInt32LE(local.length + filename.length + bytes.length, 16);
  return Buffer.concat([local, filename, bytes, central, filename, end]);
}
