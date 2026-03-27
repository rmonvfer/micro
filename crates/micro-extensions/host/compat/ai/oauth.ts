// What `@earendil-works/pi-ai/oauth` and `@mariozechner/pi-ai/oauth` resolve to.
//
// pi-ai's real `oauth.ts` is `export type` only — a type-only re-export of
// `OAuthAuthInfo`/`OAuthCredentials`/`OAuthDeviceCodeInfo`/`OAuthLoginCallbacks`/
// `OAuthPrompt`/`OAuthSelectOption`/`OAuthSelectPrompt` for coding-agent extension OAuth
// declarations. Every import from it a real extension writes is therefore `import type`,
// erased by Bun before module resolution runs — the same as every other type-only import
// from this SDK. This file matches that exactly: no runtime exports, because upstream has
// none either. It still needs to exist on disk (`crates/micro-extensions/src/compat.rs`
// includes it at compile time), so it is a real, deliberately empty module rather than a
// missing file.
export {};
