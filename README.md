# "use prompt"

Add compile-time GenAI to any SWC-powered project!

```tsx
function CoolButton() {
  "use prompt: a button that changes its background color when clicked";
}

export default function Home() {
  return <CoolButton />;
}
```

## Installation

1. Add the package as a dependency:
   ```console
   $ pnpm add swc-plugin-use-prompt
   ```
2. Add the plugin to your `next.config.{ts,js}` or `.swcrc`

   ```js
   // next.config.js
   const nextConfig = {
     experimental: {
       swcPlugins: [["swc-plugin-use-prompt", {}]],
     },
   };
   ```

   or

   ```jsonc
   // .swcrc
   {
     "jsc": {
       "experimental": {
         "plugins": [["swc-plugin-use-prompt", {}]],
       },
     },
   }
   ```

3. Run `pnpm use-prompt` alongside your regular build step!

Check out [the Next.js example](./examples/nextjs-app-router-basic) to see it in action.

## Motivation

Inspired by the [new `"use cache"` directive](https://nextjs.org/docs/canary/app/api-reference/directives)
and relentless improvements in generative AI, this project's existence felt necessary.

## How It Works

Generation happens in two passes: [the CLI script](./scripts/use-prompt.mjs) and [the SWC plugin](./src/lib.rs).

The `use-prompt` CLI script uses `@swc/core` to parse through source code and
identify all `"use prompt:"` directives. It extracts the prompts and calls
OpenAI's API to generate code snippets, which are then saved in a cache file.

At compile time, the SWC plugin reads from this cache file and modifies the AST
to insert the generated code before compilation is complete. There is some
additional complexity around dealing with package import naming clashes, which
is handled by substituting unique names for all imports required by a generated
function / component.

Ideally, codegen would happen in a single pass. However, the SWC plugin runs in
a WASM/WASI sandbox that doesn't have access to the network, so it's not
possible to call remote APIs directly. This may be changed in the future though,
since the SWC plugin API is very experimental.
