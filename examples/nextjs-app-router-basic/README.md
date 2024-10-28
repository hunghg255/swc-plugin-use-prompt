# Example with Next.js

[![Edit swc-plugin-use-prompt](https://codesandbox.io/static/img/play-codesandbox.svg)](https://codesandbox.io/p/devbox/swc-plugin-use-prompt-m73dsf?embed=1)

## Setup

1. Bootstrap a Next.js project and add this plugin.

   ```console
   $ pnpm create next-app --yes
   ✔ What is your project named? … cool-demo

   # ...

   $ cd cool-demo
   $ pnpm add swc-plugin-use-prompt
   ```

2. Add the plugin to your `next.config.ts`
   ```js
   const nextConfig = {
     experimental: {
       swcPlugins: [["swc-plugin-use-prompt", {}]],
     },
   };
   ```
3. Add your OpenAI key to the `OPENAI_API_KEY` environment variable (possibly using a `.env` file)
4. Run `pnpm dev` and `pnpm use-client` in parallel.
