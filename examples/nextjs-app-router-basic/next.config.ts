import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  /* config options here */
  experimental: {
    swcPlugins: [["swc-plugin-use-prompt", {}]],
  },
};

export default nextConfig;
