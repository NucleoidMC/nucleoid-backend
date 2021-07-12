use uuid::Uuid;
use bson::Bson;

pub fn uuid_to_bson(uuid: &Uuid) -> bson::ser::Result<Bson> {
    let serializer = bson::ser::Serializer::new();
    bson::serde_helpers::uuid_as_binary::serialize(uuid, serializer)
}
