const MEETING_HOSTS: &[&str] = &[
    "zoom.us",
    "meet.google.com",
    "teams.microsoft.com",
    "teams.live.com",
    "webex.com",
    "whereby.com",
];

const SCHEME: &str = "https://";

pub fn meeting_url(
    url: Option<&str>,
    location: Option<&str>,
    notes: Option<&str>,
) -> Option<String> {
    url.map(str::trim)
        .filter(|url| is_meeting_url(url))
        .map(str::to_owned)
        .or_else(|| {
            [location, notes]
                .into_iter()
                .flatten()
                .find_map(first_meeting_link)
        })
}

fn first_meeting_link(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    lower
        .match_indices(SCHEME)
        .map(|(start, _)| link_at(&text[start..]))
        .find(|link| is_meeting_url(link))
        .map(str::to_owned)
}

fn link_at(text: &str) -> &str {
    let end = text
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .unwrap_or(text.len());
    text[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '\''])
}

fn is_meeting_url(url: &str) -> bool {
    let Some(rest) = url
        .get(..SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
        .and_then(|_| url.get(SCHEME.len()..))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = host
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    MEETING_HOSTS.iter().any(|provider| {
        host == *provider
            || host
                .strip_suffix(provider)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use super::meeting_url;

    fn from_notes(notes: &str) -> Option<String> {
        meeting_url(None, None, Some(notes))
    }

    #[test]
    fn event_url_wins_when_it_is_a_meeting_link() {
        assert_eq!(
            meeting_url(
                Some("https://meet.google.com/abc-defg-hij"),
                Some("https://zoom.us/j/1"),
                Some("https://zoom.us/j/2"),
            )
            .as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn other_event_urls_fall_back_to_location_then_notes() {
        assert_eq!(
            meeting_url(
                Some("https://example.com/agenda"),
                Some("Room 4"),
                Some("Join: https://zoom.us/j/2"),
            )
            .as_deref(),
            Some("https://zoom.us/j/2")
        );
        assert_eq!(
            meeting_url(
                None,
                Some("https://acme.zoom.us/j/1"),
                Some("https://zoom.us/j/2"),
            )
            .as_deref(),
            Some("https://acme.zoom.us/j/1")
        );
    }

    #[test]
    fn scheme_and_host_match_without_regard_to_case() {
        assert_eq!(
            from_notes("Call HTTPS://Teams.Microsoft.com/l/meetup-join/xyz").as_deref(),
            Some("HTTPS://Teams.Microsoft.com/l/meetup-join/xyz")
        );
    }

    #[test]
    fn trailing_punctuation_and_brackets_are_trimmed() {
        assert_eq!(
            from_notes("Join here: https://whereby.com/team.").as_deref(),
            Some("https://whereby.com/team")
        );
        assert_eq!(
            from_notes("(https://acme.webex.com/meet/sam), thanks!").as_deref(),
            Some("https://acme.webex.com/meet/sam")
        );
        assert_eq!(
            from_notes("<https://teams.live.com/meet/123>").as_deref(),
            Some("https://teams.live.com/meet/123")
        );
        assert_eq!(
            from_notes("https://zoom.us/j/1?pwd=abc;").as_deref(),
            Some("https://zoom.us/j/1?pwd=abc")
        );
    }

    #[test]
    fn the_first_meeting_link_wins_and_other_links_are_skipped() {
        assert_eq!(
            from_notes(
                "Doc: https://docs.example.com/x\nZoom: https://zoom.us/j/1\nBackup: https://meet.google.com/a"
            )
            .as_deref(),
            Some("https://zoom.us/j/1")
        );
    }

    #[test]
    fn lookalike_hosts_and_plain_http_are_not_meeting_links() {
        assert_eq!(from_notes("https://notzoom.us/j/1"), None);
        assert_eq!(from_notes("https://zoom.us.example.com/j/1"), None);
        assert_eq!(from_notes("http://zoom.us/j/1"), None);
        assert_eq!(meeting_url(Some("http://zoom.us/j/1"), None, None), None);
    }

    #[test]
    fn user_info_and_ports_do_not_hide_the_host() {
        assert_eq!(from_notes("https://zoom.us@evil.example.com/j/1"), None);
        assert_eq!(
            from_notes("https://zoom.us:443/j/1").as_deref(),
            Some("https://zoom.us:443/j/1")
        );
    }

    #[test]
    fn no_link_gives_none() {
        assert_eq!(meeting_url(None, None, None), None);
        assert_eq!(
            meeting_url(None, Some("Room 4"), Some("Bring slides")),
            None
        );
        assert_eq!(from_notes("https://"), None);
    }
}
