

#[derive(Eq, PartialEq)]
pub enum AttributeType {
    HealthPoints,
    ManaPoints,
}

pub struct Attribute {
    pub attribute_type: AttributeType,
    pub max_value: f32,
    pub min_value: f32,
    pub current_value: f32,
}