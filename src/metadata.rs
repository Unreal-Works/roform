use crate::model::ModelMaterial;
use rbx_dom_weak::{Instance, WeakDom, types::Ref};
use rbx_reflection::{DataType, ReflectionDatabase};
use rbx_types::{CFrame, Variant, Vector3};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) fn material_for(dom: &WeakDom, instance: &Instance) -> ModelMaterial {
    let material_value = instance.properties.get(&"Material".into());
    let material_name = material_value
        .and_then(enum_value)
        .map(material_name)
        .unwrap_or_else(|| "plastic".to_owned());
    let color = instance
        .properties
        .get(&"Color".into())
        .or_else(|| instance.properties.get(&"Color3uint8".into()))
        .and_then(color_value)
        .unwrap_or([255, 255, 255]);
    let transparency = instance
        .properties
        .get(&"Transparency".into())
        .and_then(float_value)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);

    let mut material = ModelMaterial {
        name: material_name,
        color: [
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
            1.0 - transparency,
        ],
        base_color_asset: None,
        normal_asset: None,
    };

    if let Some(surface_appearance_ref) = direct_child(dom, instance, "SurfaceAppearance") {
        let surface_appearance = dom
            .get_by_ref(surface_appearance_ref)
            .expect("valid child ref");
        material.base_color_asset = property_asset_id(surface_appearance, "ColorMap");
        material.normal_asset = property_asset_id(surface_appearance, "NormalMap");
    }
    material
}

pub(crate) fn direct_child(dom: &WeakDom, instance: &Instance, class: &str) -> Option<Ref> {
    instance.children().iter().copied().find(|child_ref| {
        dom.get_by_ref(*child_ref)
            .is_some_and(|child| is_class(child, class))
    })
}

pub(crate) fn is_geometry(instance: &Instance) -> bool {
    matches!(
        instance.class.as_str(),
        "Part" | "WedgePart" | "CornerWedgePart" | "MeshPart" | "UnionOperation"
    )
}

pub(crate) fn is_class(instance: &Instance, class: &str) -> bool {
    instance.class.as_str() == class
}

pub(crate) fn reflection_database() -> &'static ReflectionDatabase<'static> {
    rbx_reflection_database::get().unwrap_or_else(|_| rbx_reflection_database::get_bundled())
}

pub(crate) fn roblox_instance_extras(
    dom: &WeakDom,
    instance_ref: Ref,
    database: &ReflectionDatabase<'static>,
) -> Value {
    let Some(instance) = dom.get_by_ref(instance_ref) else {
        return Value::Null;
    };

    let (properties, serialized_properties, property_types) =
        discovered_properties(instance, database);
    let children = instance
        .children()
        .iter()
        .map(|child_ref| roblox_instance_extras(dom, *child_ref, database))
        .collect::<Vec<_>>();

    let mut extras = Map::new();
    extras.insert("className".to_owned(), json!(instance.class.as_str()));
    extras.insert("name".to_owned(), json!(instance.name));
    extras.insert(
        "referent".to_owned(),
        json!(instance.referent().to_string()),
    );
    extras.insert("properties".to_owned(), Value::Object(properties));
    if !serialized_properties.is_empty() {
        extras.insert(
            "serializedProperties".to_owned(),
            Value::Object(serialized_properties),
        );
    }
    if !property_types.is_empty() {
        extras.insert("propertyTypes".to_owned(), Value::Object(property_types));
    }
    if !children.is_empty() {
        extras.insert("children".to_owned(), Value::Array(children));
    }
    Value::Object(extras)
}

fn discovered_properties(
    instance: &Instance,
    database: &ReflectionDatabase<'static>,
) -> (Map<String, Value>, Map<String, Value>, Map<String, Value>) {
    let class_descriptor = database.classes.get(instance.class.as_str());
    let mut property_names = BTreeSet::new();
    let mut reflected_property_types = Map::new();

    if let Some(class_descriptor) = class_descriptor {
        for descriptor in database.superclasses_iter(class_descriptor) {
            for (name, property) in &descriptor.properties {
                let name = (*name).to_owned();
                property_names.insert(name.clone());
                reflected_property_types
                    .insert(name, json!(reflection_data_type_name(&property.data_type)));
            }
        }
    }

    for name in instance.properties.keys() {
        let name = name.to_string();
        property_names.insert(name.clone());
    }

    let mut properties = Map::new();
    let mut serialized_properties = Map::new();
    let mut property_types = Map::new();
    for name in property_names {
        let Some(value) = instance.properties.get(&name.clone().into()) else {
            continue;
        };
        let is_default = class_descriptor
            .and_then(|class_descriptor| database.find_default_property(class_descriptor, &name))
            .is_some_and(|default| value == default);
        if is_default {
            continue;
        }

        let value_json = variant_to_json(value);
        properties.insert(name.clone(), value_json.clone());
        serialized_properties.insert(name.clone(), value_json);
        property_types.insert(
            name.clone(),
            reflected_property_types
                .get(&name)
                .cloned()
                .unwrap_or_else(|| json!(variant_type_name(value))),
        );
    }

    (properties, serialized_properties, property_types)
}

fn reflection_data_type_name(data_type: &DataType<'_>) -> String {
    match data_type {
        DataType::Value(variant_type) => format!("{variant_type:?}"),
        DataType::Enum(enum_name) => format!("Enum<{enum_name}>"),
        _ => "Unknown".to_owned(),
    }
}

fn variant_type_name(value: &Variant) -> String {
    format!("{:?}", value.ty())
}

fn variant_to_json(value: &Variant) -> Value {
    serde_json::to_value(value).unwrap_or_else(|error| {
        json!({
            "type": variant_type_name(value),
            "debug": format!("{value:?}"),
            "serializationError": error.to_string()
        })
    })
}

pub(crate) fn property_asset_id(instance: &Instance, property: &str) -> Option<String> {
    let value = instance.properties.get(&property.into())?;
    let uri = match value {
        Variant::Content(content) => content.as_uri(),
        Variant::ContentId(content_id) => Some(content_id.as_str()),
        _ => None,
    }?;
    parse_asset_id(uri)
}

fn parse_asset_id(uri: &str) -> Option<String> {
    let query_id = uri
        .split_once("id=")
        .map(|(_, value)| value)
        .and_then(|value| value.split(['&', '#', '/']).next());
    let scheme_id = uri
        .strip_prefix("rbxassetid://")
        .or_else(|| uri.strip_prefix("rbxasset://"))
        .and_then(|value| value.split(['?', '#', '/']).next());
    query_id
        .or(scheme_id)
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
        .map(str::to_owned)
}

pub(crate) fn property_cframe(instance: &Instance, property: &str) -> Option<CFrame> {
    match instance.properties.get(&property.into())? {
        Variant::CFrame(cframe) => Some(*cframe),
        Variant::OptionalCFrame(cframe) => *cframe,
        _ => None,
    }
}

pub(crate) fn property_vector3(instance: &Instance, property: &str) -> Option<Vector3> {
    match instance.properties.get(&property.into())? {
        Variant::Vector3(vector) => Some(*vector),
        _ => None,
    }
}

fn color_value(value: &Variant) -> Option<[u8; 3]> {
    match value {
        Variant::Color3uint8(color) => Some([color.r, color.g, color.b]),
        Variant::Color3(color) => Some([
            (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]),
        _ => None,
    }
}

fn float_value(value: &Variant) -> Option<f32> {
    match value {
        Variant::Float32(value) => Some(*value),
        Variant::Float64(value) => Some(*value as f32),
        _ => None,
    }
}

pub(crate) fn enum_value(value: &Variant) -> Option<u32> {
    match value {
        Variant::Enum(value) => Some(value.to_u32()),
        Variant::EnumItem(value) => Some(value.value),
        Variant::Int32(value) => (*value >= 0).then_some(*value as u32),
        Variant::Int64(value) => (*value >= 0).then_some(*value as u32),
        _ => None,
    }
}

fn material_name(value: u32) -> String {
    let name = match value {
        256 => "plastic",
        272 => "smoothplastic",
        288 => "neon",
        512 => "wood",
        528 => "woodplanks",
        784 => "marble",
        788 => "basalt",
        800 => "slate",
        804 => "crackedlava",
        816 => "concrete",
        820 => "limestone",
        832 => "granite",
        836 => "pavement",
        848 => "brick",
        864 => "pebble",
        880 => "cobblestone",
        896 => "rock",
        912 => "sandstone",
        1040 => "corrodedmetal",
        1056 => "diamondplate",
        1072 => "foil",
        1088 => "metal",
        1280 => "grass",
        1284 => "leafygrass",
        1296 => "sand",
        1312 => "fabric",
        1328 => "snow",
        1344 => "mud",
        1360 => "ground",
        1376 => "asphalt",
        1392 => "salt",
        1536 => "ice",
        1552 => "glacier",
        1568 => "glass",
        1584 => "forcefield",
        1792 => "air",
        2048 => "water",
        2304 => "cardboard",
        2305 => "carpet",
        2306 => "ceramictiles",
        2307 => "clayrooftiles",
        2308 => "roofshingles",
        2309 => "leather",
        2310 => "plaster",
        2311 => "rubber",
        _ => "plastic",
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    #[test]
    fn parses_asset_ids_from_roblox_urls() {
        assert_eq!(parse_asset_id("rbxassetid://123"), Some("123".to_owned()));
        assert_eq!(
            parse_asset_id("https://www.roblox.com/asset/?id=456"),
            Some("456".to_owned())
        );
        assert_eq!(parse_asset_id("rbxassetid://not-an-id"), None);
    }

    #[test]
    fn recognizes_primitive_part_classes_as_geometry() {
        for class in ["Part", "WedgePart", "CornerWedgePart"] {
            let dom = WeakDom::new(InstanceBuilder::new(class));
            assert!(is_geometry(dom.root()), "{class} should be geometry");
        }
    }

    #[test]
    fn discovers_serialized_unknown_and_inherited_properties() {
        let dom = WeakDom::new(
            InstanceBuilder::new("Part")
                .with_name("MetadataPart")
                .with_property("Anchored", true)
                .with_property("CustomMetadata", "kept")
                .with_child(InstanceBuilder::new("Folder").with_name("Child")),
        );
        let extras = roblox_instance_extras(&dom, dom.root_ref(), reflection_database());

        assert_eq!(extras["className"], "Part");
        assert_eq!(extras["properties"]["Anchored"]["Bool"], true);
        assert_eq!(
            extras["serializedProperties"]["CustomMetadata"]["String"],
            "kept"
        );
        assert_eq!(extras["propertyTypes"]["Anchored"], "Bool");
        assert_eq!(extras["children"][0]["name"], "Child");
    }

    #[test]
    fn omits_properties_equal_to_reflection_defaults() {
        let dom = WeakDom::new(
            InstanceBuilder::new("Part")
                .with_property("Anchored", false)
                .with_property("CustomMetadata", "kept"),
        );
        let extras = roblox_instance_extras(&dom, dom.root_ref(), reflection_database());

        assert!(extras["properties"].get("Anchored").is_none());
        assert!(extras["serializedProperties"].get("Anchored").is_none());
        assert!(extras["propertyTypes"].get("Anchored").is_none());
        assert_eq!(extras["properties"]["CustomMetadata"]["String"], "kept");
    }
}
