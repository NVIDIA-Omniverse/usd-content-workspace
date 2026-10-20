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
import { type ApiInstance } from "@/services/api";
import { fromSnakeCaseSchema } from "@/util/schemas";

export function thumbnailService(api: ApiInstance) {
  return {
    async getThumbnails(
      resourceAddress: string,
      {
        width,
        height,
        encodingFormat,
        transformType,
        cropStrategy,
        specification,
      }: GetThumbnailsOptions = {},
    ): Promise<GetThumbnailsResponse> {
      const search = new URLSearchParams();
      search.set("association_target_resource_address", resourceAddress);
      if (width !== undefined) search.set("width", width.toString());
      if (height !== undefined) search.set("height", height.toString());
      if (encodingFormat !== undefined)
        search.set("encoding_format", encodingFormat);
      if (transformType !== undefined)
        search.set("transform_type", transformType);
      if (cropStrategy !== undefined) search.set("crop_strategy", cropStrategy);
      if (specification !== undefined)
        search.set("specification", specification);

      const response = await api.get(`v1alpha/thumbnails?${search.toString()}`);
      const data = await response.json();
      return GetThumbnailsResponseSchema.parse(data);
    },
  };
}

export type ThumbnailEncodingFormat = "JPEG" | "PNG" | "GIF" | "WEBP";

export type ThumbnailTransformType =
  | "SQUARE"
  | "SCALE_BY_WIDTH"
  | "SCALE_BY_HEIGHT"
  | "CROP"
  | "FIT"
  | "SCALE_AND_CROP"
  | "THUMBNAIL"
  | "RESIZE";

export type ThumbnailCropStrategy =
  | "CENTER"
  | "TOP"
  | "BOTTOM"
  | "LEFT"
  | "RIGHT"
  | "TOP_LEFT"
  | "TOP_RIGHT"
  | "BOTTOM_LEFT"
  | "BOTTOM_RIGHT"
  | "SMART";

export type ThumbnailSpecificationType = "IMAGE_TO_IMAGE";

/** Exact-match filters; any that are omitted do not restrict the result. */
export interface GetThumbnailsOptions {
  width?: number;
  height?: number;
  encodingFormat?: ThumbnailEncodingFormat;
  transformType?: ThumbnailTransformType;
  cropStrategy?: ThumbnailCropStrategy;
  specification?: ThumbnailSpecificationType;
}

const ResourceRefSchema = fromSnakeCaseSchema(
  z.object({
    resourceAddress: z.string(),
    resourceIdentity: z.string(),
  }),
);

const ThumbnailEntrySchema = fromSnakeCaseSchema(
  z.object({
    thumbnail: ResourceRefSchema,
    // Shape varies by specification type and has nullable members, so it is
    // passed through untouched rather than validated.
    specification: z.unknown().nullable().optional(),
    createTime: z.string().nullable().optional(),
  }),
);
export type ThumbnailEntry = z.infer<typeof ThumbnailEntrySchema>;

const GetThumbnailsResponseSchema = fromSnakeCaseSchema(
  z.object({
    thumbnails: z.array(ThumbnailEntrySchema),
  }),
);
export type GetThumbnailsResponse = z.infer<typeof GetThumbnailsResponseSchema>;
