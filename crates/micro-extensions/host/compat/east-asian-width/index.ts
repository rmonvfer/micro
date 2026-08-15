
export function eastAsianWidth(codepoint: number): number {
	if (isWide(codepoint)) {
		return 2;
	}
	return 1;
}

function isWide(cp: number): boolean {
	return (
		(cp >= 0x1100 && cp <= 0x115f) || 
		(cp >= 0x2e80 && cp <= 0x303e) || 
		(cp >= 0x3041 && cp <= 0x33ff) || 
		(cp >= 0x3400 && cp <= 0x4dbf) || 
		(cp >= 0x4e00 && cp <= 0x9fff) || 
		(cp >= 0xa000 && cp <= 0xa4cf) || 
		(cp >= 0xac00 && cp <= 0xd7a3) || 
		(cp >= 0xf900 && cp <= 0xfaff) || 
		(cp >= 0xfe30 && cp <= 0xfe4f) || 
		(cp >= 0xff00 && cp <= 0xff60) || 
		(cp >= 0xffe0 && cp <= 0xffe6) || 
		(cp >= 0x20000 && cp <= 0x3fffd) 
	);
}
