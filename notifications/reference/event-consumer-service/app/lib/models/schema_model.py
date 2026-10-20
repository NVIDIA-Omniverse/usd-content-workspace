# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Base models for REST API models with schema utilities."""

from typing import Any

from pydantic import BaseModel


class SchemaModel(BaseModel):
    """Base model with schema manipulation utilities for OpenAPI integration."""

    @classmethod
    def simplified_schema(cls) -> dict:
        """
        Simplify the schema to a version supported by OpenAPI.

        Resolves all $ref references by inlining the definitions from $defs,
        removing the need for separate definition sections.

        Returns:
            The schema with all $ref references resolved and $defs removed
        """
        schema = cls.model_json_schema()
        return cls._resolve_schema_refs(schema)

    @classmethod
    def add_to_openapi_schema(cls, openapi_schema: dict) -> None:
        """
        Add this model and its dependencies to an OpenAPI schema.

        This method:
        1. Extracts $defs from the model's JSON schema
        2. Adds them to the OpenAPI components/schemas
        3. Converts the main model schema
        4. Replaces $defs references with component references

        Args:
            openapi_schema: The OpenAPI schema dictionary to modify (in place)
        """
        # Ensure components/schemas exists
        if "components" not in openapi_schema:
            openapi_schema["components"] = {}
        if "schemas" not in openapi_schema["components"]:
            openapi_schema["components"]["schemas"] = {}

        model_schema = cls.model_json_schema()
        model_name = cls.__name__

        # Extract and merge $defs into components/schemas
        for schema_name, schema_def in model_schema.pop("$defs", {}).items():
            if schema_name not in openapi_schema["components"]["schemas"]:
                openapi_schema["components"]["schemas"][schema_name] = schema_def

        # Replace $defs references with component references
        cls._replace_defs_with_component_refs(model_schema)
        openapi_schema["components"]["schemas"][model_name] = model_schema

    @staticmethod
    def _resolve_schema_refs(schema: dict) -> dict:
        """
        Resolve $ref references in a JSON schema by inlining definitions.

        Args:
            schema: The JSON schema dictionary to resolve

        Returns:
            The schema with all $ref references resolved and $defs removed
        """
        defs = schema.get("$defs", {})

        def resolve(obj: Any) -> Any:
            if isinstance(obj, dict):
                if "$ref" in obj:
                    ref_key = obj["$ref"].split("/")[-1]
                    ref_obj = defs.get(ref_key, {})
                    return resolve(ref_obj)

                return {k: resolve(v) for k, v in obj.items()}
            elif isinstance(obj, list):
                return [resolve(item) for item in obj]
            else:
                return obj

        return {k: resolve(v) for k, v in schema.items() if k != "$defs"}

    @staticmethod
    def _replace_defs_with_component_refs(obj: Any) -> None:
        """
        Replace $defs references with #/components/schemas references in place.

        Args:
            obj: The object to modify (dict or list). Modified in place.
        """
        if isinstance(obj, dict):
            if "$ref" in obj and obj["$ref"].startswith("#/$defs/"):
                # Change #/$defs/X to #/components/schemas/X
                obj["$ref"] = obj["$ref"].replace("#/$defs/", "#/components/schemas/")
            for value in obj.values():
                SchemaModel._replace_defs_with_component_refs(value)
        elif isinstance(obj, list):
            for item in obj:
                SchemaModel._replace_defs_with_component_refs(item)
