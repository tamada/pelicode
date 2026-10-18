//! Rough SPDX identification of license texts, for repositories whose
//! metadata does not declare a license (Go modules).

/// The SPDX identifier of a well-known license text, if recognized.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::license::identify;
///
/// let text = "MIT License\n\nPermission is hereby granted, free of charge, to any person ...";
/// assert_eq!(identify(text), Some("MIT"));
/// assert_eq!(identify("All rights reserved."), None);
/// ```
pub fn identify(text: &str) -> Option<&'static str> {
    let t: String = text
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let has = |s: &str| t.contains(s);
    Some(if has("apache license") && has("version 2.0") {
        "Apache-2.0"
    } else if has("permission is hereby granted, free of charge") {
        "MIT"
    } else if has("redistribution and use in source and binary forms") {
        if has("neither the name") || has("the names of its contributors may not") {
            "BSD-3-Clause"
        } else {
            "BSD-2-Clause"
        }
    } else if has("permission to use, copy, modify, and/or distribute this software") {
        "ISC"
    } else if has("mozilla public license version 2.0") || has("mozilla public license, v. 2.0") {
        "MPL-2.0"
    } else if has("gnu lesser general public license") && has("version 3") {
        "LGPL-3.0-only"
    } else if has("gnu general public license") && has("version 3") {
        "GPL-3.0-only"
    } else if has("gnu general public license") && has("version 2") {
        "GPL-2.0-only"
    } else if has("this is free and unencumbered software released into the public domain") {
        "Unlicense"
    } else {
        return None;
    })
}

/// The license of a package from its license files; several distinct ones are joined with `AND`.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::license::identify_all;
///
/// let texts = vec![
///     "Apache License\nVersion 2.0, January 2004".to_string(),
///     "Permission is hereby granted, free of charge, to any person".to_string(),
/// ];
/// assert_eq!(identify_all(&texts).as_deref(), Some("Apache-2.0 AND MIT"));
/// ```
pub fn identify_all(texts: &[String]) -> Option<String> {
    let mut ids: Vec<&str> = texts.iter().filter_map(|t| identify(t)).collect();
    ids.sort();
    ids.dedup();
    (!ids.is_empty()).then(|| ids.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_common_licenses() {
        let mit = "MIT License\n\nPermission is hereby granted, free of charge, to any person";
        assert_eq!(identify(mit), Some("MIT"));
        let bsd3 = "Redistribution and use in source and binary forms, with or without\n\
                    modification ... * Neither the name of Google Inc. nor";
        assert_eq!(identify(bsd3), Some("BSD-3-Clause"));
        assert_eq!(
            identify("Apache License\n  Version 2.0, January 2004"),
            Some("Apache-2.0")
        );
        assert_eq!(identify("all rights reserved"), None);
        assert_eq!(
            identify_all(&[mit.into(), mit.into(), bsd3.into()]).as_deref(),
            Some("BSD-3-Clause AND MIT")
        );
    }
}
