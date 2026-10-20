# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Tests for the SchemaModel base class."""

from pydantic import BaseModel

from app.lib.models.schema_model import SchemaModel


class TestSimplifiedSchema:
    def test_resolves_ref_to_inline_definition(self):
        class InnerType(BaseModel):
            value: str

        class TestModel(SchemaModel):
            field: InnerType

        result = TestModel.simplified_schema()

        # The $ref should be resolved inline
        assert "$defs" not in result
        assert result["properties"]["field"]["type"] == "object"
        assert "value" in result["properties"]["field"]["properties"]

    def test_removes_defs_from_result(self):
        class TestModel(SchemaModel):
            name: str

        result = TestModel.simplified_schema()

        assert "$defs" not in result

    def test_handles_nested_models(self):
        class InnerModel(BaseModel):
            value: str

        class MiddleModel(BaseModel):
            inner: InnerModel

        class TestModel(SchemaModel):
            middle: MiddleModel

        result = TestModel.simplified_schema()

        # All nested models should be inlined, not referenced
        assert "$defs" not in result
        assert "$ref" not in str(result)

    def test_handles_unions(self):
        class TypeA(BaseModel):
            field_a: str

        class TestModel(SchemaModel):
            data: TypeA | dict

        result = TestModel.simplified_schema()

        # Should have anyOf with both types inlined
        assert "$defs" not in result
        assert "anyOf" in result["properties"]["data"]


class TestAddToOpenApiSchema:
    def test_adds_model_to_components_schemas(self):
        class TestModel(SchemaModel):
            name: str
            age: int

        openapi_schema = {}

        TestModel.add_to_openapi_schema(openapi_schema)

        assert "components" in openapi_schema
        assert "schemas" in openapi_schema["components"]
        assert "TestModel" in openapi_schema["components"]["schemas"]

    def test_extracts_and_adds_nested_models(self):
        class InnerModel(BaseModel):
            value: str

        class OuterModel(SchemaModel):
            inner: InnerModel

        openapi_schema = {}

        OuterModel.add_to_openapi_schema(openapi_schema)

        # The InnerModel should be extracted from $defs and added
        assert "InnerModel" in openapi_schema["components"]["schemas"]
        assert "OuterModel" in openapi_schema["components"]["schemas"]

    def test_replaces_defs_refs_with_component_refs(self):
        class InnerModel(BaseModel):
            value: str

        class OuterModel(SchemaModel):
            inner: InnerModel

        openapi_schema = {}

        OuterModel.add_to_openapi_schema(openapi_schema)

        # Check that the reference was updated
        inner_ref = openapi_schema["components"]["schemas"]["OuterModel"]["properties"][
            "inner"
        ]["$ref"]
        assert inner_ref == "#/components/schemas/InnerModel"

    def test_does_not_duplicate_shared_models(self):
        class SharedModel(BaseModel):
            value: str

        class ModelA(SchemaModel):
            shared: SharedModel

        class ModelB(SchemaModel):
            shared: SharedModel

        openapi_schema = {}

        ModelA.add_to_openapi_schema(openapi_schema)
        ModelB.add_to_openapi_schema(openapi_schema)

        # SharedModel should only appear once
        schemas = openapi_schema["components"]["schemas"]
        shared_count = list(schemas.keys()).count("SharedModel")
        assert shared_count == 1

    def test_preserves_existing_components(self):
        openapi_schema = {
            "components": {"schemas": {"ExistingModel": {"type": "object"}}}
        }

        class NewModel(SchemaModel):
            name: str

        NewModel.add_to_openapi_schema(openapi_schema)

        # Both models should be present
        assert "ExistingModel" in openapi_schema["components"]["schemas"]
        assert "NewModel" in openapi_schema["components"]["schemas"]

    def test_multiple_models_can_add_themselves(self):
        class ModelA(SchemaModel):
            field_a: str

        class ModelB(SchemaModel):
            field_b: int

        openapi_schema = {}

        ModelA.add_to_openapi_schema(openapi_schema)
        ModelB.add_to_openapi_schema(openapi_schema)

        assert "ModelA" in openapi_schema["components"]["schemas"]
        assert "ModelB" in openapi_schema["components"]["schemas"]
