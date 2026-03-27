// pi-tui's `utils.ts` measures a codepoint's terminal cell width by asking the real
// `get-east-asian-width` npm package, which classifies every Unicode codepoint against
// the East Asian Width property table. That table runs to thousands of ranges — not
// something worth reproducing here down to the codepoint. What is reproduced is the
// property's practical effect: the ranges a terminal actually renders two columns wide
// (CJK ideographs and their punctuation, fullwidth forms, and a handful of symbol blocks
// commonly rendered double-width). A codepoint outside them reports one column, which is
// the right answer for the overwhelming majority of text — Latin, Cyrillic, Greek, Arabic,
// digits, punctuation — and a plausible approximation rather than a silent miscount for
// the rest.
export function eastAsianWidth(codepoint: number): number {
	if (isWide(codepoint)) {
		return 2;
	}
	return 1;
}

function isWide(cp: number): boolean {
	return (
		(cp >= 0x1100 && cp <= 0x115f) || // Hangul Jamo
		(cp >= 0x2e80 && cp <= 0x303e) || // CJK Radicals, Kangxi, CJK symbols/punctuation
		(cp >= 0x3041 && cp <= 0x33ff) || // Hiragana .. CJK Compatibility
		(cp >= 0x3400 && cp <= 0x4dbf) || // CJK Extension A
		(cp >= 0x4e00 && cp <= 0x9fff) || // CJK Unified Ideographs
		(cp >= 0xa000 && cp <= 0xa4cf) || // Yi
		(cp >= 0xac00 && cp <= 0xd7a3) || // Hangul Syllables
		(cp >= 0xf900 && cp <= 0xfaff) || // CJK Compatibility Ideographs
		(cp >= 0xfe30 && cp <= 0xfe4f) || // CJK Compatibility Forms
		(cp >= 0xff00 && cp <= 0xff60) || // Fullwidth Forms
		(cp >= 0xffe0 && cp <= 0xffe6) || // Fullwidth Signs
		(cp >= 0x20000 && cp <= 0x3fffd) // CJK Extension B and beyond, and supplementary ideographs
	);
}
