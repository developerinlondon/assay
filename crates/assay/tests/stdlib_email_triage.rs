mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_email_triage() {
    let script = r#"
        local mod = require("assay.email_triage")
        assert.not_nil(mod)
        assert.not_nil(mod.categorize)
        assert.not_nil(mod.categorize_llm)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_needs_action() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "ceo@example.com", subject = "Action required: budget sign-off" },
            { from = "pm@example.com", subject = "URGENT deadline changed" },
        })
        assert.eq(#result.needs_action, 2)
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_needs_reply() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "alice@example.com", subject = "Can we meet tomorrow?" },
            { from = "bob@example.com", subject = "Question about rollout" },
        })
        assert.eq(#result.needs_reply, 2)
        assert.eq(result.needs_reply[1].from, "alice@example.com")
        assert.eq(#result.needs_action, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_fyi() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({
            { from = "noreply@example.com", subject = "Your weekly report" },
            { from = "alerts@example.com", subject = "Automated deployment notice", automated = true },
        })
        assert.eq(#result.fyi, 2)
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.needs_action, 0)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_categorize_empty() {
    let script = r#"
        local triage = require("assay.email_triage")
        local result = triage.categorize({})
        assert.eq(#result.needs_reply, 0)
        assert.eq(#result.needs_action, 0)
        assert.eq(#result.fyi, 0)
    "#;
    run_lua(script).await.unwrap();
}

// ---------------------------------------------------------------------------
// M.signals: what a message says about itself, with no model and no network.
//
// Every fixture is a message shape, so these tests are pure Lua — there is no
// vendor here to stand a server in front of. 2026-03-02T09:00:00Z is passed as
// `now` throughout, because "the next 14 September" is not a fixed date.
// ---------------------------------------------------------------------------

/// 2026-03-02T09:00:00Z.
const MARCH: i64 = 1_772_442_000;

fn signals(msg: &str, body: &str) -> String {
    format!(
        "local t = require(\"assay.email_triage\")\n\
         local s = t.signals({msg}, {{ now = {MARCH} }})\n{body}"
    )
}

#[tokio::test]
async fn test_a_message_declaring_itself_a_machine_is_read_as_one() {
    let headers = [
        (
            r#"{ ["Auto-Submitted"] = "auto-replied" }"#,
            "Auto-Submitted: auto-replied",
        ),
        (
            r#"{ ["auto-submitted"] = "AUTO-GENERATED" }"#,
            "auto-generated",
        ),
        (r#"{ ["X-Autoreply"] = "yes" }"#, "x-autoreply"),
        (r#"{ ["X-Autorespond"] = "yes" }"#, "x-autorespond"),
        (
            r#"{ ["X-Auto-Response-Suppress"] = "All" }"#,
            "x-auto-response-suppress",
        ),
        (
            r#"{ ["Precedence"] = "auto_reply" }"#,
            "Precedence: auto_reply",
        ),
    ];
    for (header, evidence) in headers {
        let msg =
            format!(r#"{{ headers = {header}, subject = "Re: intro", text = "I am away." }}"#);
        let body = format!(
            r#"
            assert.eq(s.auto_reply.present, true)
            assert.contains(s.auto_reply.evidence, "{evidence}")
            "#
        );
        run_lua(&signals(&msg, &body)).await.unwrap();
    }
}

/// The header rides on ordinary mail too, and there it says the opposite.
#[tokio::test]
async fn test_auto_submitted_no_is_a_person_writing() {
    let msg =
        r#"{ headers = { ["Auto-Submitted"] = "no" }, subject = "Re: intro", text = "Sure." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.auto_reply.present, false)
        assert.eq(s.auto_reply.evidence, nil)
        "#,
    ))
    .await
    .unwrap();
}

/// `Precedence: bulk` is what a mailing list sets, and a person can write from
/// one. Counted as an auto-reply it would silence every reply that happened to
/// come through a list, so it is reported beside the verdict rather than as one.
#[tokio::test]
async fn test_precedence_bulk_is_noted_beside_the_verdict_and_never_as_one() {
    let msg =
        r#"{ headers = { Precedence = "bulk" }, subject = "Re: intro", text = "Sounds good." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.auto_reply.present, false)
        assert.eq(s.auto_reply.bulk, true)
        "#,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_an_auto_reply_subject_is_read_in_every_language_the_lists_cover() {
    let subjects = [
        "Automatic reply: Re: intro",
        "Automatische Antwort: Re: intro",
        "Réponse automatique : Re: intro",
        "Respuesta automática: Re: intro",
    ];
    for subject in subjects {
        let msg = format!(r#"{{ headers = {{}}, subject = "{subject}", text = "." }}"#);
        run_lua(&signals(&msg, "assert.eq(s.auto_reply.present, true)"))
            .await
            .unwrap();
    }
}

/// Case and accents cannot hide a phrase: `string.lower` is byte-wise ASCII, so
/// "BÜRO" lowercases to something that never matches "buro".
#[tokio::test]
async fn test_an_away_message_is_read_in_every_language_the_lists_cover() {
    let bodies = [
        ("I am out of office until 14 September.", "2026-09-14"),
        ("Ich bin bis zum 14. SEPTEMBER nicht im BÜRO.", "2026-09-14"),
        (
            "Je suis absent du bureau, de retour le 3 avril.",
            "2026-04-03",
        ),
        (
            "Estoy FUERA DE LA OFICINA hasta el 14 de septiembre.",
            "2026-09-14",
        ),
    ];
    for (text, until) in bodies {
        let msg = format!(r#"{{ headers = {{}}, subject = "Re: intro", text = "{text}" }}"#);
        let body = format!(
            r#"
            assert.eq(s.out_of_office.present, true)
            assert.not_nil(s.out_of_office.evidence)
            assert.eq(s.out_of_office["until"], "{until}")
            "#
        );
        run_lua(&signals(&msg, &body)).await.unwrap();
    }
}

#[tokio::test]
async fn test_the_day_they_said_they_are_back_is_read_either_way_round() {
    let script = format!(
        r#"
        local t = require("assay.email_triage")
        assert.eq(t.return_date("back on 14 September", {MARCH}), "2026-09-14")
        assert.eq(t.return_date("back on September 14th", {MARCH}), "2026-09-14")
        assert.eq(t.return_date("away until the 3rd of April", {MARCH}), "2026-04-03")
        assert.eq(t.return_date("bis zum 14. September", {MARCH}), "2026-09-14")
        "#
    );
    run_lua(&script).await.unwrap();
}

/// Read as this year, a date already gone sends the follow-up today, into an
/// inbox nobody is reading.
#[tokio::test]
async fn test_a_day_already_gone_this_year_means_the_next_one() {
    let script = format!(
        r#"
        local t = require("assay.email_triage")
        assert.eq(t.return_date("back on 1 January", {MARCH}), "2027-01-01")
        assert.eq(t.return_date("back on 1 March", {MARCH}), "2027-03-01")
        "#
    );
    run_lua(&script).await.unwrap();
}

/// They said they are back today, so they are back today. Pushed a year on, the
/// campaign waits twelve months for somebody sitting at their desk.
#[tokio::test]
async fn test_a_day_that_is_today_is_today() {
    let script = format!(
        r#"
        local t = require("assay.email_triage")
        assert.eq(t.return_date("back on 2 March", {MARCH}), "2026-03-02")
        "#
    );
    run_lua(&script).await.unwrap();
}

/// A date the calendar does not have is not a date. Rolled forward it becomes
/// the first of March, which is a day they never named.
#[tokio::test]
async fn test_a_date_the_calendar_does_not_have_is_no_date_at_all() {
    let script = format!(
        r#"
        local t = require("assay.email_triage")
        assert.eq(t.return_date("back on 31 February", {MARCH}), nil)
        assert.eq(t.return_date("back on 31 April", {MARCH}), nil)
        assert.eq(t.return_date("back soon", {MARCH}), nil)
        assert.eq(t.return_date("", {MARCH}), nil)
        -- No month is thirty-two days long, so no number that large is a day,
        -- whether it is a typo or a year sitting beside a month name.
        assert.eq(t.return_date("back on 32 March", {MARCH}), nil)
        assert.eq(t.return_date("away until September 2026", {MARCH}), nil)
        assert.eq(t.return_date("since 2026 we have", {MARCH}), nil)
        "#
    );
    run_lua(&script).await.unwrap();
}

/// An away message that names 29 February in a year that has no 29 February is
/// naming nothing, and the next leap year is not what they meant.
#[tokio::test]
async fn test_a_leap_day_is_read_only_in_a_year_that_has_one() {
    let script = format!(
        r#"
        local t = require("assay.email_triage")
        -- 2026 is not a leap year, and the roll-forward lands on 2027, which is
        -- not one either.
        assert.eq(t.return_date("back on 29 February", {MARCH}), nil)
        "#
    );
    run_lua(&script).await.unwrap();
}

/// Cold outreach carries an unsubscribe line on every send, and a reply quotes
/// it underneath. Matched there, every reply anyone ever sends reads as somebody
/// asking to be left alone.
#[tokio::test]
async fn test_only_the_senders_own_words_are_searched() {
    let quoted = [
        "Sounds good.\\n\\nOn Tue, we wrote:\\n> To unsubscribe, click here",
        "Sounds good.\\n\\n----- Original Message -----\\nTo unsubscribe, click here",
        "Sounds good.\\n\\n> please remove me from your list",
        "Klingt gut.\\n\\nAm Dienstag schrieb wir:\\nAbmelden hier",
    ];
    for text in quoted {
        let msg = format!(r#"{{ headers = {{}}, subject = "Re: intro", text = "{text}" }}"#);
        run_lua(&signals(&msg, "assert.eq(s.unsubscribe.present, false)"))
            .await
            .unwrap();
    }
}

/// A client that quotes with no marker leaves a header block instead, written in
/// whatever language it runs in. Knowing English and German only, the reader
/// takes a French or Dutch forward's quoted original for the sender's own words
/// — and the original carries the unsubscribe line the campaign sent.
#[tokio::test]
async fn test_a_quoted_header_block_is_a_quote_in_every_language_the_lists_know() {
    let forwards = [
        // French Outlook: De / Envoye / A / Objet.
        "Sounds good, count me in.\\n\\nDe : Marie Dubois <marie@exemple.fr>\\n\
         Envoyé : mardi 3 mars 2026 09:12\\nÀ : ceo@ours.test\\n\
         Objet : RE: distribution\\n\\nPlease unsubscribe me from this list.",
        // Dutch Outlook: Van / Verzonden / Aan / Onderwerp.
        "Sounds good, count me in.\\n\\nVan: Jan de Vries <jan@voorbeeld.nl>\\n\
         Verzonden: dinsdag 3 maart 2026 09:12\\nAan: ceo@ours.test\\n\
         Onderwerp: RE: distribution\\n\\nPlease unsubscribe me from this list.",
    ];
    for text in forwards {
        let script = format!(
            "local t = require(\"assay.email_triage\")\n\
             local text = \"{text}\"\n\
             assert.eq(t.own_words(text), \"Sounds good, count me in.\\n\")\n\
             local s = t.signals(\
               {{ headers = {{}}, subject = \"Re: distribution\", text = text }}, \
               {{ now = {MARCH} }})\n\
             assert.eq(s.unsubscribe.present, false)"
        );
        run_lua(&script).await.unwrap();
    }
}

/// One line naming a sender is not a quote block: a reply may perfectly well
/// open "From: our end, this looks fine". Cut there, everything the sender went
/// on to say is thrown away — here, the opt-out the message exists to make.
#[tokio::test]
async fn test_a_lone_sender_line_is_not_a_quote_block() {
    let bodies = [
        "From: our end, this looks fine.\\n\\nDo not contact me again about this.",
        "From: ola@fjord.test is the address you asked for.\\n\\n\
         Do not contact me again about this.",
        // The same two under a line of their own, so a cut would leave
        // something and the whole-message fallback cannot stand in for the
        // guard being tested.
        "Thanks for the note.\\nFrom: our end, this looks fine.\\n\\n\
         Do not contact me again about this.",
        // An address on the line, so only the window rule is left standing
        // between this reply and a cut that loses the opt-out.
        "Thanks for the note.\\nFrom: ola@fjord.test is the address you asked for.\\n\\n\
         Do not contact me again about this.",
        // A second header-shaped line right under the first, so only the
        // address on the sender line tells prose from a quoted block.
        "Thanks for the note.\\nFrom: our end, this looks fine.\\n\
         To: be clear, we are not interested.\\n\\n\
         Do not contact me again about this.",
        // A header-shaped line far below is a coincidence, not the rest of a
        // block: a window wide enough to reach it cuts a reply in half.
        "Thanks for the note.\\nFrom: ola@fjord.test can help.\\n\\n\\n\\n\\n\
         To: be clear, we are not interested.\\n\\n\
         Do not contact me again about this.",
    ];
    for text in bodies {
        let script = format!(
            "local t = require(\"assay.email_triage\")\n\
             local text = \"{text}\"\n\
             assert.eq(t.own_words(text), text)\n\
             local s = t.signals(\
               {{ headers = {{}}, subject = \"Re: distribution\", text = text }}, \
               {{ now = {MARCH} }})\n\
             assert.eq(s.unsubscribe.present, true)"
        );
        run_lua(&script).await.unwrap();
    }
}

/// A cut that leaves nothing did not find a quote, it found the whole message:
/// a forward typed out by hand opens with the block. Handed on as an empty
/// message it says nothing, and the one thing it came to say is the referral.
#[tokio::test]
async fn test_a_cut_that_would_leave_nothing_keeps_the_whole_message() {
    let text = "De : Marie Dubois <marie@exemple.fr>\\nObjet : RE: distribution\\n\\n\
                Please contact Marie about this.";
    let script = format!(
        "local t = require(\"assay.email_triage\")\n\
         local text = \"{text}\"\n\
         assert.eq(t.own_words(text), text)\n\
         local s = t.signals(\
           {{ headers = {{}}, subject = \"Re: distribution\", text = text }}, \
           {{ now = {MARCH} }})\n\
         assert.eq(s.referral.present, true)"
    );
    run_lua(&script).await.unwrap();
}

/// The header is put on outbound mail. A reply that quotes the letter carries it
/// back, so counting it would read a compliant campaign's own footer as the
/// recipient's request.
#[tokio::test]
async fn test_list_unsubscribe_is_noted_and_never_counted() {
    let msg = r#"{ headers = { ["List-Unsubscribe"] = "<https://x.test/u>" },
                   subject = "Re: intro", text = "Sounds good." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.unsubscribe.present, false)
        assert.eq(s.unsubscribe.list_unsubscribe, true)
        "#,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_someone_asking_to_be_left_alone_is_read_in_every_language() {
    let bodies = [
        "Please remove me from your list.",
        "Don’t contact me again.",
        "Bitte abmelden, keine weiteren E-Mails.",
        "Merci de ne plus me contacter.",
        "Por favor, quiero darme de baja.",
    ];
    for text in bodies {
        let msg = format!(r#"{{ headers = {{}}, subject = "Re: intro", text = "{text}" }}"#);
        run_lua(&signals(
            &msg,
            r#"
            assert.eq(s.unsubscribe.present, true)
            assert.not_nil(s.unsubscribe.evidence)
            "#,
        ))
        .await
        .unwrap();
    }
}

/// A delivery report says what it is in its content type, and names the address
/// it could not reach in the machine-readable part below the human one.
#[tokio::test]
async fn test_a_delivery_report_is_read_from_its_content_type_and_names_the_address() {
    let msg = r#"{ headers = { ["Content-Type"] = "multipart/report; report-type=delivery-status; boundary=b" },
                   subject = "Delivery Status Notification (Failure)",
                   text = "Final-Recipient: rfc822; gone@example.test\nStatus: 5.1.1" }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.bounce.present, true)
        assert.contains(s.bounce.evidence, "delivery-status")
        assert.eq(s.bounce.address, "gone@example.test")
        "#,
    ))
    .await
    .unwrap();
}

/// Some reports say nothing a phrase list would catch. The sender alone decides
/// these: no human writes from the daemon.
#[tokio::test]
async fn test_a_bounce_from_the_daemon_is_read_from_its_sender_alone() {
    for from in [
        "MAILER-DAEMON@mx.example.test",
        "postmaster@mx.example.test",
    ] {
        // Deliberately carries no bounce phrase and no report content type, so
        // only the sender can be what decided it.
        let msg = format!(
            r#"{{ headers = {{ From = "{from}" }}, subject = "Re: intro",
                   text = "550 5.1.1 <gone@example.test>" }}"#
        );
        run_lua(&signals(
            &msg,
            r#"
            assert.eq(s.bounce.present, true)
            assert.contains(s.bounce.evidence, "From: ")
        "#,
        ))
        .await
        .unwrap();
    }
}

/// A report has no sender's words to cut at: the machine-readable part sits
/// below the quoted original, and reading only the top throws the evidence away.
#[tokio::test]
async fn test_a_bounce_is_read_from_the_whole_body_and_not_only_its_top() {
    let msg = r#"{ headers = {}, subject = "Re: intro",
                   text = "Delivery to the following recipient failed.\n\n> Original message\n> Subject: intro\n\nFinal-Recipient: rfc822; gone@example.test\nDiagnostic-Code: smtp; 550 user unknown" }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.bounce.present, true)
        assert.eq(s.bounce.address, "gone@example.test")
        "#,
    ))
    .await
    .unwrap();
}

/// The phrases are ones a person writing prose does not produce by accident. A
/// reply about a department that does not exist is not a bounce, which is why
/// that wording is deliberately not on the list.
#[tokio::test]
async fn test_ordinary_prose_is_not_a_bounce() {
    let bodies = [
        "That team does not exist any more, but I can help.",
        "Our mailbox is quite full at the moment, apologies for the delay.",
        "Send me the deck.",
    ];
    for text in bodies {
        let msg = format!(r#"{{ headers = {{}}, subject = "Re: intro", text = "{text}" }}"#);
        run_lua(&signals(&msg, "assert.eq(s.bounce.present, false)"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_a_referral_carries_the_address_it_points_at() {
    let msg = r#"{ headers = {}, subject = "Re: intro",
                   text = "Not me I'm afraid. Please contact Dana at dana@example.test." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.referral.present, true)
        assert.eq(s.referral.address, "dana@example.test")
        assert.contains(s.referral.evidence, "Dana")
        "#,
    ))
    .await
    .unwrap();
}

/// The first address in a reply is usually a signature block. Pointing a
/// campaign at a signature is worse than pointing it nowhere, so the address
/// comes from the sentence that made the referral.
#[tokio::test]
async fn test_a_referral_takes_the_address_from_the_sentence_and_not_the_signature() {
    let msg = r#"{ headers = {}, subject = "Re: intro",
                   text = "Please contact Dana at dana@example.test.\n\n--\nJo Church\njo@example.test" }"#;
    run_lua(&signals(
        msg,
        r#"assert.eq(s.referral.address, "dana@example.test")"#,
    ))
    .await
    .unwrap();
}

/// A referral with no address named is still a referral; it just points at a
/// person rather than an inbox.
#[tokio::test]
async fn test_a_referral_without_an_address_is_still_a_referral() {
    let msg = r#"{ headers = {}, subject = "Re: intro",
                   text = "You should speak to our head of ops about this." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.referral.present, true)
        assert.eq(s.referral.address, nil)
        "#,
    ))
    .await
    .unwrap();
}

/// Nothing is inferred from a signal not firing, so an ordinary reply is read as
/// exactly that: a message none of the five had anything to say about.
#[tokio::test]
async fn test_an_ordinary_reply_fires_nothing_and_says_so_for_every_signal() {
    let msg = r#"{ headers = {}, subject = "Re: North American distribution",
                   text = "Thanks, this is timely. Send me the deck." }"#;
    run_lua(&signals(
        msg,
        r#"
        for _, name in ipairs({ "auto_reply", "bounce", "out_of_office", "unsubscribe", "referral" }) do
          assert.not_nil(s[name])
          assert.eq(s[name].present, false)
          assert.eq(s[name].evidence, nil)
        end
        "#,
    ))
    .await
    .unwrap();
}

/// The five are independent and none ranks the others. A message can be an away
/// notice that also asks to be left alone, and which of those wins is the
/// caller's policy rather than this module's.
#[tokio::test]
async fn test_the_signals_are_independent_and_none_ranks_the_others() {
    let msg = r#"{ headers = { ["Auto-Submitted"] = "auto-replied" }, subject = "Re: intro",
                   text = "I am out of office until 14 September. Also please unsubscribe me." }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.auto_reply.present, true)
        assert.eq(s.out_of_office.present, true)
        assert.eq(s.unsubscribe.present, true)
        assert.eq(s.out_of_office["until"], "2026-09-14")
        "#,
    ))
    .await
    .unwrap();
}

/// A message with only an HTML part still says what it says. Read as nothing,
/// every HTML-only away message would be invisible.
#[tokio::test]
async fn test_an_html_only_message_is_still_read() {
    let msg = r#"{ headers = {}, subject = "Re: intro",
                   html = "<p>I am <b>out of office</b> until 14 September.</p>" }"#;
    run_lua(&signals(
        msg,
        r#"
        assert.eq(s.out_of_office.present, true)
        assert.eq(s.out_of_office["until"], "2026-09-14")
        -- Tag removal leaves the gaps the tags stood in; the quote is still one
        -- a reader would recognise.
        assert.contains(s.out_of_office.evidence, "out of office")
        "#,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_a_header_is_found_whatever_case_its_name_was_written_in() {
    run_lua(
        r#"
        local t = require("assay.email_triage")
        local h = { ["AUTO-Submitted"] = "auto-replied", ["x-AutoReply"] = "yes", Empty = "  " }
        assert.eq(t.header(h, "auto-submitted"), "auto-replied")
        assert.eq(t.header(h, "X-AUTOREPLY"), "yes")
        -- A header present but empty says nothing, and reads as absent.
        assert.eq(t.header(h, "empty"), nil)
        assert.eq(t.header(h, "missing"), nil)
        assert.eq(t.header(nil, "anything"), nil)
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_accents_and_case_cannot_hide_a_phrase() {
    run_lua(
        r#"
        local t = require("assay.email_triage")
        assert.eq(t.fold("Nicht im BÜRO"), "nicht im buro")
        assert.eq(t.fold("AUSSER HAUS"), "ausser haus")
        assert.eq(t.fold("Je suis ABSENT DU BUREAU"), "je suis absent du bureau")
        assert.eq(t.fold("Respuesta automática"), "respuesta automatica")
        -- Mail clients substitute the curly apostrophe silently.
        assert.eq(t.fold("Don’t"), "don't")
        assert.eq(t.fold("groß"), "gross")
        "#,
    )
    .await
    .unwrap();
}

/// The older pass keeps working: this is a function beside it, not a
/// replacement, and a caller on it is not moved by this release.
#[tokio::test]
async fn test_the_older_categorize_is_untouched() {
    run_lua(
        r#"
        local t = require("assay.email_triage")
        local out = t.categorize({
          { from = "ceo@example.test", subject = "Action required: budget" },
          { from = "noreply@example.test", subject = "Newsletter" },
          { from = "jo@example.test", subject = "Re: intro" },
        })
        assert.eq(#out.needs_action, 1)
        assert.eq(#out.fyi, 1)
        assert.eq(#out.needs_reply, 1)
        "#,
    )
    .await
    .unwrap();
}

/// A message that is not a table at all is a caller's mistake, and answering it
/// with five absences is a truthful reading of nothing rather than a crash.
#[tokio::test]
async fn test_a_message_that_is_not_a_message_reads_as_nothing_rather_than_crashing() {
    run_lua(
        r#"
        local t = require("assay.email_triage")
        for _, bad in ipairs({ "a string", 42, true }) do
          local s = t.signals(bad)
          assert.eq(s.unsubscribe.present, false)
          assert.eq(s.bounce.present, false)
        end
        local s = t.signals(nil)
        assert.eq(s.out_of_office.present, false)
        "#,
    )
    .await
    .unwrap();
}
