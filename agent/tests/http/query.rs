use miru_agent::http::query::{Page, QueryParams};

pub mod query_params {
    use super::*;

    #[test]
    fn empty() {
        let qp = QueryParams::new();
        assert_eq!(qp.into_pairs(), vec![]);
    }

    #[test]
    fn pagination_only() {
        let pagination = Page {
            limit: 10,
            offset: 0,
        };
        let qp = QueryParams::new().paginate(&pagination);
        assert_eq!(
            qp.into_pairs(),
            vec![
                ("limit".to_string(), "10".to_string()),
                ("offset".to_string(), "0".to_string()),
            ]
        );
    }

    #[test]
    fn pagination_with_expand() {
        let pagination = Page {
            limit: 10,
            offset: 0,
        };
        let qp = QueryParams::new()
            .paginate(&pagination)
            .expand(["test", "test2"]);
        assert_eq!(
            qp.into_pairs(),
            vec![
                ("limit".to_string(), "10".to_string()),
                ("offset".to_string(), "0".to_string()),
                ("expand".to_string(), "test".to_string()),
                ("expand".to_string(), "test2".to_string()),
            ]
        );
    }

    #[test]
    fn pagination_with_add() {
        let pagination = Page {
            limit: 10,
            offset: 0,
        };
        let qp = QueryParams::new()
            .paginate(&pagination)
            .add("activity_status", "active,inactive");
        assert_eq!(
            qp.into_pairs(),
            vec![
                ("limit".to_string(), "10".to_string()),
                ("offset".to_string(), "0".to_string()),
                ("activity_status".to_string(), "active,inactive".to_string()),
            ]
        );
    }

    #[test]
    fn combined() {
        let pagination = Page {
            limit: 100,
            offset: 0,
        };
        let qp = QueryParams::new()
            .paginate(&pagination)
            .add("activity_status", "active")
            .expand(["current_release", "device"]);
        assert_eq!(
            qp.into_pairs(),
            vec![
                ("limit".to_string(), "100".to_string()),
                ("offset".to_string(), "0".to_string()),
                ("activity_status".to_string(), "active".to_string()),
                ("expand".to_string(), "current_release".to_string()),
                ("expand".to_string(), "device".to_string()),
            ]
        );
    }

    #[test]
    fn expand_only() {
        let qp = QueryParams::new().expand(["test"]);
        assert_eq!(
            qp.into_pairs(),
            vec![("expand".to_string(), "test".to_string())]
        );
    }

    #[test]
    fn expand_empty_slice() {
        let qp = QueryParams::new().expand(Vec::<&str>::new());
        assert_eq!(qp.into_pairs(), vec![]);
    }

    #[test]
    fn pagination_default() {
        let qp = QueryParams::new().paginate(&Page::default());
        assert_eq!(
            qp.into_pairs(),
            vec![
                ("limit".to_string(), "10".to_string()),
                ("offset".to_string(), "0".to_string()),
            ]
        );
    }
}
