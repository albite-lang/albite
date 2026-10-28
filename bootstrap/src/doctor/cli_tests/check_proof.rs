//! Which `doctor check proof` spellings clap accepts (ADR 30.9.26f D2,
//! 1.10.26m, 2.10.26o, 3.10.26s). Split from `grouping.rs` when 3.10.26s's
//! flag pushed that file past its function-count threshold: the proof
//! sub-namespace's flags move together, so the next one lands here.

#[cfg(test)]
mod check_proof_cli_tests {
    use crate::doctor::CheckCommands;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        cmd: CheckCommands,
    }

    fn parse(args: &[&str]) -> Result<TestCli, clap::Error> {
        TestCli::try_parse_from(std::iter::once("test").chain(args.iter().copied()))
    }

    /// 30.9.26f AC4: the census parses where the tree says, with and without
    /// `--include-total`, and the flag lands on the variant.
    #[test]
    fn test_check_proof_kernel_census_parses() {
        use crate::doctor::check_commands::check_proof::CheckProofCommands;
        use crate::doctor::CheckCommands;

        // 1.10.26m D1: `--all-rows` parses on its own and lands on the variant.
        for (args, want_total, want_all_rows) in [
            (&["proof", "kernel-census", "test.tg"][..], false, false),
            (
                &["proof", "kernel-census", "test.tg", "--include-total"][..],
                true,
                false,
            ),
            (
                &["proof", "kernel-census", "test.tg", "--all-rows"][..],
                false,
                true,
            ),
        ] {
            let parsed = parse(args).expect("parses");
            let CheckCommands::Proof(CheckProofCommands::KernelCensus {
                file: Some(file),
                include_total,
                all_rows,
                ..
            }) = parsed.cmd
            else {
                panic!("{args:?} did not parse to kernel-census");
            };
            assert_eq!(file, std::path::PathBuf::from("test.tg"));
            assert_eq!((include_total, all_rows), (want_total, want_all_rows));
        }
        assert!(parse(&["proof", "kernel-census"]).is_err());
        assert!(parse(&["proof", "no-such-check", "test.tg"]).is_err());
    }

    /// 2.10.26o AC5: the corpus form takes one or more roots and no file; a
    /// bare `kernel-census`, a file beside `--corpus`, and `--corpus` beside
    /// `--include-total` or `--all-rows` are all refused (D1, D5).
    #[test]
    fn test_check_proof_kernel_census_corpus_flags_parse() {
        use crate::doctor::check_commands::check_proof::CheckProofCommands;
        use crate::doctor::CheckCommands;

        let parsed = parse(&["proof", "kernel-census", "--corpus", "a", "b"]).expect("parses");
        let CheckCommands::Proof(CheckProofCommands::KernelCensus { file, corpus, .. }) =
            parsed.cmd
        else {
            panic!("--corpus did not parse to kernel-census");
        };
        assert_eq!(file, None);
        assert_eq!(
            corpus,
            vec![std::path::PathBuf::from("a"), std::path::PathBuf::from("b")]
        );

        let parsed = parse(&["proof", "kernel-census", "test.tg"]).expect("parses");
        let CheckCommands::Proof(CheckProofCommands::KernelCensus { corpus, .. }) = parsed.cmd
        else {
            panic!("a file did not parse to kernel-census");
        };
        assert!(corpus.is_empty());

        for refused in [
            &["proof", "kernel-census"][..],
            &["proof", "kernel-census", "test.tg", "--corpus", "a"][..],
            &["proof", "kernel-census", "--corpus", "a", "--include-total"][..],
            &["proof", "kernel-census", "--corpus", "a", "--all-rows"][..],
            &["proof", "kernel-census", "--corpus"][..],
        ] {
            assert!(parse(refused).is_err(), "{refused:?} parsed");
        }
    }

    /// 3.10.26s AC4: `--proof-bearing` parses beside `--corpus` and is a clap
    /// error without it (D1).
    #[test]
    fn test_check_proof_kernel_census_proof_bearing_is_corpus_only() {
        use crate::doctor::check_commands::check_proof::CheckProofCommands;
        use crate::doctor::CheckCommands;

        let args = ["proof", "kernel-census", "--corpus", "a", "--proof-bearing"];
        let parsed = parse(&args).expect("parses");
        let CheckCommands::Proof(CheckProofCommands::KernelCensus { proof_bearing, .. }) =
            parsed.cmd
        else {
            panic!("--proof-bearing did not parse to kernel-census");
        };
        assert!(proof_bearing);
        assert!(parse(&["proof", "kernel-census", "test.tg", "--proof-bearing"]).is_err());
        assert!(parse(&["proof", "kernel-census", "--proof-bearing"]).is_err());
    }

    /// 15.10.26g AC1 (D2, D6): `--all-definitions` excludes `--include-total`,
    /// the thresholds need `--baseline`, and `--top` does not.
    #[test]
    fn test_check_proof_term_size_flag_constraints() {
        use crate::doctor::check_commands::check_proof::CheckProofCommands;
        use crate::doctor::CheckCommands;

        let args = [
            "proof",
            "term-size",
            "m.tg",
            "--all-definitions",
            "--baseline",
            "b.json",
            "--min-delta",
            "7",
            "--max-growth",
            "1.25",
        ];
        let parsed = parse(&args).expect("parses");
        let CheckCommands::Proof(CheckProofCommands::TermSize {
            all_definitions,
            baseline,
            min_delta,
            max_growth,
            ..
        }) = parsed.cmd
        else {
            panic!("did not parse to term-size");
        };
        assert!(all_definitions);
        assert_eq!(baseline, Some(std::path::PathBuf::from("b.json")));
        assert_eq!((min_delta, max_growth), (Some(7), Some(1.25)));
        assert!(parse(&["proof", "term-size", "m.tg", "--top", "3"]).is_ok());
        for refused in [
            &[
                "proof",
                "term-size",
                "m.tg",
                "--include-total",
                "--all-definitions",
            ][..],
            &["proof", "term-size", "m.tg", "--min-delta", "5"][..],
            &["proof", "term-size", "m.tg", "--max-growth", "2"][..],
        ] {
            assert!(parse(refused).is_err(), "{refused:?} parsed");
        }
    }
}
