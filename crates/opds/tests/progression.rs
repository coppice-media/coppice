use stump_core::opds::v2_0::{
	link::{OPDSLink, OPDSLinkFinalizer},
	progression::{OPDS_PROGRESSION_MEDIA_TYPE, OPDS_PROGRESSION_REL},
};

#[test]
fn progression_discovery_advertises_the_stable_contract_on_the_existing_route() {
	let finalizer = OPDSLinkFinalizer::new("https://books.example.test".to_string());
	let link =
		serde_json::to_value(OPDSLink::progression("book-1".into(), &finalizer)).unwrap();
	assert_eq!(
		link["href"],
		"https://books.example.test/opds/v2.0/books/book-1/progression"
	);
	assert_eq!(link["rel"], OPDS_PROGRESSION_REL);
	assert_eq!(link["type"], OPDS_PROGRESSION_MEDIA_TYPE);
	assert_eq!(
		link["properties"]["authenticate"]["href"],
		"https://books.example.test/opds/v2.0/auth"
	);
}
