export type ThumbPixels = { width: number; height: number; rgba: Uint8ClampedArray<ArrayBuffer> };
export function decodeThumbHash(value: string): ThumbPixels | undefined {
    let hash: Uint8Array;
    try { hash = Uint8Array.from(atob(value), character => character.charCodeAt(0)); } catch { return; }
    if (hash.length < 5 || hash.length > 100) return;
    const header = hash[0] | hash[1] << 8 | hash[2] << 16, dimensions = hash[3] | hash[4] << 8;
    const alpha = Boolean(header & 1 << 23), landscape = Boolean(dimensions & 1 << 15), limit = alpha ? 5 : 7;
    const lx = Math.max(3, landscape ? limit : dimensions & 7), ly = Math.max(3, landscape ? dimensions & 7 : limit);
    let index = 0;
    const start = alpha ? 6 : 5;
    const channel = (nx: number, ny: number, scale: number) => {
        const coefficients: { x: number; y: number; value: number }[] = [];
        for (let y = 0; y < ny; y++) for (let x = y === 0 ? 1 : 0; x * ny < nx * (ny - y); x++) {
            const byte = hash[start + (index >> 1)];
            if (byte === undefined) throw new Error('Truncated ThumbHash');
            coefficients.push({ x, y, value: (((byte >> ((index++ & 1) * 4)) & 15) / 7.5 - 1) * scale });
        }
        return coefficients;
    };
    try {
        const l = channel(lx, ly, ((header >> 18) & 31) / 31), p = channel(3, 3, ((dimensions >> 3) & 63) / 63 * 1.25), q = channel(3, 3, ((dimensions >> 9) & 63) / 63 * 1.25);
        const a = alpha ? channel(5, 5, (hash[5] >> 4) / 15) : [];
        const ratio = (landscape ? limit : dimensions & 7) / (landscape ? dimensions & 7 : limit);
        if (!Number.isFinite(ratio) || ratio <= 0) return;
        const width = Math.max(1, Math.round(ratio > 1 ? 32 : 32 * ratio)), height = Math.max(1, Math.round(ratio > 1 ? 32 / ratio : 32));
        const rgba = new Uint8ClampedArray(width * height * 4);
        const cosineX=Array.from({length:width},(_,x)=>Array.from({length:7},(_,frequency)=>Math.cos(Math.PI*(x+.5)*frequency/width)));
        const cosineY=Array.from({length:height},(_,y)=>Array.from({length:7},(_,frequency)=>Math.cos(Math.PI*(y+.5)*frequency/height)));
        for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
            const sum = (coefficients: typeof l, dc: number) => coefficients.reduce((value, coefficient) => value + 2 * coefficient.value * cosineX[x][coefficient.x] * cosineY[y][coefficient.y], dc);
            const light = sum(l, (header & 63) / 63), blueYellow = sum(p, ((header >> 6) & 63) / 31.5 - 1), redGreen = sum(q, ((header >> 12) & 63) / 31.5 - 1);
            const blue = light - blueYellow * 2 / 3, red = (3 * light - blue + redGreen) / 2;
            const offset = (y * width + x) * 4;
            rgba[offset] = red * 255; rgba[offset + 1] = (red - redGreen) * 255; rgba[offset + 2] = blue * 255; rgba[offset + 3] = (alpha ? sum(a, (hash[5] & 15) / 15) : 1) * 255;
        }
        return { width, height, rgba };
    } catch { return; }
}
