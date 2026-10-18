/**
 * COCO shapes for the label viewer — a TS mirror of the Rust
 * `vlm_orchestrator::auto_label::package_coco` (docs/09 §4): 1-indexed
 * image/annotation ids, bbox [x, y, w, h] in image pixels, area = w·h,
 * iscrowd 0, fail-closed taxonomy.
 */

export interface CocoImage {
  id: number;
  file_name: string;
  height: number;
  width: number;
}

export interface CocoAnnotation {
  id: number;
  image_id: number;
  category_id: number;
  bbox: [number, number, number, number];
  area: number;
  iscrowd: 0;
}

export interface CocoDataset {
  images: CocoImage[];
  annotations: CocoAnnotation[];
  categories: string[];
}

/** Frame bytes endpoint (the ONLY route that serves jpeg_lowres). */
export function imageUrl(frameId: string): string {
  return `/api/v1/snapshots/image/${encodeURIComponent(frameId)}`;
}

/** One operator-drawn box in normalized [0..1] frame coords. */
export interface DraftBox {
  x: number;
  y: number;
  w: number;
  h: number;
  category: string;
}

interface PackageInput {
  frameId: string;
  fileName: string;
  width: number;
  height: number;
  boxes: DraftBox[];
  categories: string[];
}

/** Pixel-space COCO skeleton; ids sequence 1.. like the Rust pack. */
export function packageCoco(input: PackageInput): CocoDataset {
  const images: CocoImage[] = [
    { id: 1, file_name: input.fileName, height: input.height, width: input.width },
  ];
  const annotations: CocoAnnotation[] = [];
  let annId = 1;
  for (const box of input.boxes) {
    const categoryIndex = input.categories.indexOf(box.category);
    if (categoryIndex < 0) continue; // fail-closed taxonomy
    const w = Math.round(box.w * input.width);
    const h = Math.round(box.h * input.height);
    const x = Math.round(box.x * input.width);
    const y = Math.round(box.y * input.height);
    annotations.push({
      id: annId,
      image_id: 1,
      category_id: categoryIndex + 1,
      bbox: [x, y, w, h],
      area: w * h,
      iscrowd: 0,
    });
    annId += 1;
  }
  return { images, annotations, categories: [...input.categories] };
}

/** Approve POST body extension carrying the fixed annotations. */
export function cocoApproveBody(operatorId: string, coco: CocoDataset | null): {
  operator_id: string;
  coco_annotations: CocoDataset | null;
} {
  return { operator_id: operatorId, coco_annotations: coco };
}
