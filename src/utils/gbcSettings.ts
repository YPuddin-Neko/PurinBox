export interface GbcOptions {
  python_path: string;
  detector_path: string;
  embedder_path: string;
  device: string;
  content_type: string;
  caption_prompt: string;
  max_nodes: number;
  max_depth: number;
  max_requests: number;
}
export const GBC_DEFAULTS: GbcOptions = {
  python_path: '', detector_path: '', embedder_path: '', device: 'cpu',
  content_type: 'caption', caption_prompt: '', max_nodes: 20, max_depth: 3, max_requests: 64,
};
