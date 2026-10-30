# Beans marketing design

The owner pins an Apple-inspired bento layout and a product interaction inspired by Openbot. Preserve the approved coral SVG identity and self-hosted Manrope. The website persuades visitors through working illustrative controls, provider compatibility and explicit free pricing.

Use a spacious editorial opening, a wide chat demo, and an asymmetric six-column bento grid. Cards vary in scale and contain readable task artefacts: conversations, file changes, memory notes, schedules and connection details. The product work is the imagery. Use soft off-white surfaces in light appearance and charcoal surfaces in dark appearance, coral for action and selective emphasis, 16px card corners, 24px interior spacing and shared responsive page gutters. Do not restore the removed capability ticker.

Use staggered scroll assembly for the bento, timed message reveals in the demo, and restrained hover depth. Content starts visible. All motion obeys reduced-motion, pause and visibility controls. Mobile cards become one column and the chat sidebar condenses into a team header. Comparisons use readable tables and sources, with no invented absence claims.

## Preflight: 2026-10-29

**The bet:** Beans puts a team of persistent bots on devices people already own, with their existing providers and no app fee.
**Variable:** download conversion, with documentation supporting successful setup.
**Audience:** product-aware visitors comparing local, multi-bot and provider-flexible alternatives.
**Not to be done:** keep the single-page homepage and current routes; preserve the coral identity, Manrope, product demo, framework, translations and factual claims.
**Approved foundation:** the spacious editorial opening, Openbot-inspired product interaction and asymmetric bento above.
**Delivery:** one polished responsive implementation, reviewed locally in both appearances.

| Check | Status |
| --- | --- |
| Brand source of truth | Approved coral SVG masters and this file agree |
| Loadable fonts | Self-hosted Manrope files ship with the site |
| Photography | Not applicable; real product UI and brand geometry are the imagery |
| Brand files | Vector light/dark logos, marks and favicons exist in `web/public/brand` |
| Sourced numbers | Keep the existing $0 app-price claim and provider-cost explanation; add no metrics |

## Surface classification

| Route | Class | Business variable | Persuasion |
| --- | --- | --- | --- |
| `/` and localized homepages | A acquisition | Download conversion | Promise, interactive product proof, provider compatibility and free pricing |
| `/compare` and localized indexes | A acquisition | Qualified comparison visits and downloads | Concrete tradeoffs, costs and dedicated detail links |
| `/compare/<product>` and localized details | A acquisition | Decision confidence and downloads | Evidence, readable comparison dimensions, sources and selection guidance |
| `/docs/<page>` and `/zh/docs/<page>` | B work | Setup completion and product activation | Low-noise navigation, search, readable instructions and copyable examples |

## Design direction

- **Product:** Beans gives people persistent AI teammates that work on their own computers with their existing providers.
- **Character:** warm, precise, quietly playful.
- **World:** a tidy physical desk with one sculpted coral object and a working screen.
- **Moments:** the tactile bean sculpture in the hero and the real app preview; pricing and the closing action use the same coral with confidence.
- **Signature:** compact coral section indices and rules guide the editorial page without competing with the product.

Build a tactile editorial system from Beans' own geometry: oversized type, offset section headers, layered neutral surfaces and one clear motion idea per section. Coral belongs to the brand object, key figures, selection and action. Keep chrome quiet, use native scrolling and transform/opacity animation, preserve visible server-rendered content, and keep the pause and reduced-motion controls authoritative. Meaningful task demonstrations may animate; decorative breathing, ambient washes and pointer glows do not belong. Docs share the palette and graphic details without acquiring marketing motion.

## Scars

- Content must render before client animation modules load; never make the hero or sections depend on JavaScript for visibility.
- Preserve the WebGL SVG fallback, capped pixel ratio and offscreen/hidden-tab pauses.
- Small screens intentionally scroll the desktop preview horizontally; this must not create document-level horizontal overflow.
- Comparison tables may scroll internally, while sources and absence claims remain factual and readable.
- The closing panel needs its coral background and dark text from the same visual-system rule; the older global tint override made its copy invisible in dark mode (fixed after browser inspection, 2026-10-30).
