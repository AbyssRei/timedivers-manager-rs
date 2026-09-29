/// Sentinel branch name used in manifests/config for the default Steam branch.
pub const NONE_BRANCH: &str = "None";

/// Sentinel version date representing the original Steam installation.
pub const STEAM_VERSION: &str = "steam";

/// Reserved key inside a manifest date entry holding the patch title.
pub const PATCH_TITLE_KEY: &str = "patch_title";

/// Normalizes a beta key from `appmanifest_*.acf` to the sentinel branch name.
pub fn effective_branch(beta: &str) -> &str {
    if beta.eq_ignore_ascii_case("public") {
        NONE_BRANCH
    } else {
        beta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_beta_maps_to_none() {
        assert_eq!(effective_branch("public"), NONE_BRANCH);
        assert_eq!(effective_branch("Public"), NONE_BRANCH);
        assert_eq!(effective_branch("PUBLIC"), NONE_BRANCH);
        assert_eq!(effective_branch("None"), NONE_BRANCH);
        assert_eq!(effective_branch("beta-experimental"), "beta-experimental");
    }
}
