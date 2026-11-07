export interface QueuedCommand {
  route: string;
  events?: string[];
  cancel?: string;
}

export const QUEUED_COMMANDS: Record<string, QueuedCommand> = {
  delete_dedup_files: { route: '/image-dedup' },
  analyze_buckets: { route: '/bucket-preview', events: ['bucket-progress'], cancel: 'cancel_bucket_analysis' },
  recommend_bucket_params: { route: '/bucket-preview', cancel: 'cancel_bucket_recommend' },
  export_buckets: { route: '/bucket-preview', events: ['bucket-export-progress'] },
  scan_concept_folders: { route: '/dataset-balancer' },
  apply_concept_repeats: { route: '/dataset-balancer' },
  deploy_python_env: { route: '/settings' },
  reset_python_env: { route: '/settings' },
  download_danbooru_tags: { route: '/settings', events: ['tag-db-progress'], cancel: 'cancel_tag_db_download' },
  translate_tag_db: { route: '/settings', events: ['tag-db-progress'], cancel: 'cancel_tag_db_translation' },
  download_person_crop_model: { route: '/person-crop', events: ['person-crop-download'], cancel: 'cancel_person_crop' },
  load_tag_dataset: { route: '/tag-manager' },
  load_caption_dataset: { route: '/tag-manager' },
  save_all_tag_files: { route: '/tag-manager' },
  save_all_caption_files: { route: '/tag-manager' },
};
