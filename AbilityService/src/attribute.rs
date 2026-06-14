

#[derive(Eq, PartialEq, Debug, Copy, Clone, Hash)]
pub enum AttributeType {
    HealthPoints,
    ManaPoints,
}

#[derive(Eq, PartialEq, Debug, Hash)]
pub struct Attribute {
    pub attribute_type: AttributeType,
    pub max_value: i32,
    pub min_value: i32,
    pub current_value: i32,
}