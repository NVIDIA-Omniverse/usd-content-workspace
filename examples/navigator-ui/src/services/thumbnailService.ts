// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import { z } from "zod";
import { ApiInstance } from "@/services/api";
import { fromSnakeCaseSchema } from "@/util/schemas";

export function thumbnailService(api: ApiInstance) {
  return {
    async getThumbnails(
      resourceAddress: string,
      { signal, width, height, encodingFormat, transformType, cropDirection, specification }: GetThumbnailsOptions = {}
    ): Promise<GetThumbnailsResponse> {
      const search = new URLSearchParams();
      search.set("association_target_resource_address", resourceAddress);
      if (width !== undefined) {
        search.set("width", width.toString());
      }
      if (height !== undefined) {
        search.set("height", height.toString());
      }
      if (encodingFormat !== undefined) {
        search.set("encoding_format", encodingFormat);
      }
      if (transformType !== undefined) {
        search.set("transform_type", transformType);
      }
      if (cropDirection !== undefined) {
        search.set("crop_direction", cropDirection);
      }
      if (specification !== undefined) {
        search.set("specification", specification);
      }

      const response = await api.get(`thumbnails?${search}`, { signal });
      const data = await response.json();
      return GetThumbnailsResponseSchema.parse(data);
    },
  };
}

export const TargetType = {
  FILE: "FILE",
  FOLDER: "FOLDER",
} as const;
export type TargetType = (typeof TargetType)[keyof typeof TargetType];

export type ThumbnailEncodingFormat = "JPEG" | "PNG" | "GIF" | "WEBP";

export type ThumbnailTransformType =
  | "SQUARE"
  | "SCALE_BY_WIDTH"
  | "SCALE_BY_HEIGHT"
  | "CROP"
  | "FIT"
  | "SCALE_AND_CROP";

export type ThumbnailCropDirection =
  | "CENTER"
  | "TOP"
  | "BOTTOM"
  | "LEFT"
  | "RIGHT"
  | "TOP_LEFT"
  | "TOP_RIGHT"
  | "BOTTOM_LEFT"
  | "BOTTOM_RIGHT";

export type ThumbnailSpecificationType = "IMAGE_TO_IMAGE" | "VIDEO_TO_IMAGE" | "USD_TO_IMAGE";

export interface GetThumbnailsOptions {
  signal?: AbortSignal;
  width?: number;
  height?: number;
  encodingFormat?: ThumbnailEncodingFormat;
  transformType?: ThumbnailTransformType;
  cropDirection?: ThumbnailCropDirection;
  specification?: ThumbnailSpecificationType;
}

export const ResourceRefSchema = fromSnakeCaseSchema(
  z.object({
    resourceAddress: z.string(),
    resourceIdentity: z.string(),
  })
);

const ImageToImageSpecificationSchema = fromSnakeCaseSchema(
  z.object({
    processorIdentity: z.string().optional(),
    encodingFormat: z.string().optional(),
    width: z.number().optional(),
    height: z.number().optional(),
    transformType: z.string().optional(),
    cropDirection: z.string().optional(),
    quality: z.number().optional(),
    preserveAspectRatio: z.boolean().optional(),
  })
);

const SpecificationSchema = fromSnakeCaseSchema(
  z.object({
    imageToImage: ImageToImageSpecificationSchema.nullable().optional(),
  })
);

const GetThumbnailsResponseSchema = fromSnakeCaseSchema(
  z.object({
    thumbnails: z.array(
      z.object({
        thumbnail: ResourceRefSchema,
        specification: SpecificationSchema.nullable().optional(),
      })
    ),
  })
);
export type GetThumbnailsResponse = z.infer<typeof GetThumbnailsResponseSchema>;
