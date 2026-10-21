export class Positions {
    private heights: Float64Array;
    private tree: Float64Array;
    constructor(values: readonly number[]) {
        this.heights = Float64Array.from(values);
        this.tree = new Float64Array(values.length + 1);
        for (let i = 0; i < values.length; i++) {
            this.tree[i + 1] += values[i];
            const parent = (i + 1) + ((i + 1) & -(i + 1));
            if (parent < this.tree.length)
                this.tree[parent] += this.tree[i + 1];
        }
    }
    get length() { return this.heights.length; }
    height(index: number) { return this.heights[index] ?? 0; }
    measure(index: number, height: number) { if (index < 0 || index >= this.length || !Number.isFinite(height) || height <= 0)
        return; const delta = height - this.heights[index]; this.heights[index] = height; for (let i = index + 1; i < this.tree.length; i += i & -i)
        this.tree[i] += delta; }
    offset(index: number) { let sum = 0; for (let i = Math.min(index, this.length); i > 0; i -= i & -i)
        sum += this.tree[i]; return sum; }
    get total() { return this.offset(this.length); }
    indexAt(offset: number) { let index = 0, sum = 0; let bit = 1; while (bit * 2 < this.tree.length)
        bit *= 2; for (; bit > 0; bit >>= 1) {
        const next = index + bit;
        if (next < this.tree.length && sum + this.tree[next] <= offset) {
            index = next;
            sum += this.tree[next];
        }
    } return Math.max(0, Math.min(this.length - 1, index)); }
}
