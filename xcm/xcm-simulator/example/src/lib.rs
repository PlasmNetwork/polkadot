// Copyright 2021 Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

mod parachain;
mod relay_chain;

use frame_support::sp_tracing;
use xcm::{latest::Error, prelude::*};
use xcm_executor::traits::Convert;
use xcm_simulator::{decl_test_network, decl_test_parachain, decl_test_relay_chain};

pub const ALICE: sp_runtime::AccountId32 = sp_runtime::AccountId32::new([0u8; 32]);
pub const INITIAL_BALANCE: u128 = 1_000_000_000;

decl_test_parachain! {
	pub struct ParaA {
		Runtime = parachain::Runtime,
		XcmpMessageHandler = parachain::MsgQueue,
		DmpMessageHandler = parachain::MsgQueue,
		new_ext = para_ext(1),
	}
}

decl_test_parachain! {
	pub struct ParaB {
		Runtime = parachain::Runtime,
		XcmpMessageHandler = parachain::MsgQueue,
		DmpMessageHandler = parachain::MsgQueue,
		new_ext = para_ext(2),
	}
}

decl_test_relay_chain! {
	pub struct Relay {
		Runtime = relay_chain::Runtime,
		XcmConfig = relay_chain::XcmConfig,
		new_ext = relay_ext(),
	}
}

decl_test_network! {
	pub struct MockNet {
		relay_chain = Relay,
		parachains = vec![
			(1, ParaA),
			(2, ParaB),
		],
	}
}

pub fn parent_account_id() -> parachain::AccountId {
	let location = (Parent,);
	parachain::LocationToAccountId::convert(location.into()).unwrap()
}

pub fn child_account_id(para: u32) -> relay_chain::AccountId {
	let location = (Parachain(para),);
	relay_chain::LocationToAccountId::convert(location.into()).unwrap()
}

pub fn child_account_account_id(para: u32, who: sp_runtime::AccountId32) -> relay_chain::AccountId {
	let location = (Parachain(para), AccountId32 { network: None, id: who.into() });
	relay_chain::LocationToAccountId::convert(location.into()).unwrap()
}

pub fn sibling_account_account_id(para: u32, who: sp_runtime::AccountId32) -> parachain::AccountId {
	let location = (Parent, Parachain(para), AccountId32 { network: None, id: who.into() });
	parachain::LocationToAccountId::convert(location.into()).unwrap()
}

pub fn parent_account_account_id(who: sp_runtime::AccountId32) -> parachain::AccountId {
	let location = (Parent, AccountId32 { network: None, id: who.into() });
	parachain::LocationToAccountId::convert(location.into()).unwrap()
}

pub fn para_ext(para_id: u32) -> sp_io::TestExternalities {
	use parachain::{MsgQueue, Runtime, System};

	let mut t = frame_system::GenesisConfig::default().build_storage::<Runtime>().unwrap();

	pallet_balances::GenesisConfig::<Runtime> {
		balances: vec![(ALICE, INITIAL_BALANCE), (parent_account_id(), INITIAL_BALANCE)],
	}
	.assimilate_storage(&mut t)
	.unwrap();

	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		sp_tracing::try_init_simple();
		System::set_block_number(1);
		MsgQueue::set_para_id(para_id.into());
	});
	ext
}

pub fn relay_ext() -> sp_io::TestExternalities {
	use relay_chain::{Runtime, RuntimeOrigin, System, Uniques};

	let mut t = frame_system::GenesisConfig::default().build_storage::<Runtime>().unwrap();

	pallet_balances::GenesisConfig::<Runtime> {
		balances: vec![
			(ALICE, INITIAL_BALANCE),
			(child_account_id(1), INITIAL_BALANCE),
			(child_account_id(2), INITIAL_BALANCE),
		],
	}
	.assimilate_storage(&mut t)
	.unwrap();

	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		assert_eq!(Uniques::force_create(RuntimeOrigin::root(), 1, ALICE, true), Ok(()));
		assert_eq!(Uniques::mint(RuntimeOrigin::signed(ALICE), 1, 42, child_account_id(1)), Ok(()));
	});
	ext
}

pub type RelayChainPalletXcm = pallet_xcm::Pallet<relay_chain::Runtime>;
pub type ParachainPalletXcm = pallet_xcm::Pallet<parachain::Runtime>;

#[cfg(test)]
mod tests {
	use super::*;

	use codec::Encode;
	use frame_support::assert_ok;
	use xcm::latest::QueryResponseInfo;
	use xcm_simulator::TestExt;

	// Helper function for forming buy execution message
	fn buy_execution<C>(fees: impl Into<MultiAsset>) -> Instruction<C> {
		BuyExecution { fees: fees.into(), weight_limit: Unlimited }
	}

	#[test]
	fn remote_account_ids_work() {
		child_account_account_id(1, ALICE);
		sibling_account_account_id(1, ALICE);
		parent_account_account_id(ALICE);
	}

	#[test]
	fn dmp() {
		MockNet::reset();

		let remark = parachain::RuntimeCall::System(
			frame_system::Call::<parachain::Runtime>::remark_with_event { remark: vec![1, 2, 3] },
		);
		Relay::execute_with(|| {
			assert_ok!(RelayChainPalletXcm::send_xcm(
				Here,
				Parachain(1),
				Xcm(vec![Transact {
					origin_kind: OriginKind::SovereignAccount,
					require_weight_at_most: INITIAL_BALANCE as u64,
					call: remark.encode().into(),
				}]),
			));
		});

		ParaA::execute_with(|| {
			use parachain::{RuntimeEvent, System};
			assert!(System::events().iter().any(|r| matches!(
				r.event,
				RuntimeEvent::System(frame_system::Event::Remarked { .. })
			)));
		});
	}

	#[test]
	fn ump() {
		MockNet::reset();

		let remark = relay_chain::RuntimeCall::System(
			frame_system::Call::<relay_chain::Runtime>::remark_with_event { remark: vec![1, 2, 3] },
		);
		ParaA::execute_with(|| {
			assert_ok!(ParachainPalletXcm::send_xcm(
				Here,
				Parent,
				Xcm(vec![Transact {
					origin_kind: OriginKind::SovereignAccount,
					require_weight_at_most: INITIAL_BALANCE as u64,
					call: remark.encode().into(),
				}]),
			));
		});

		Relay::execute_with(|| {
			use relay_chain::{RuntimeEvent, System};
			assert!(System::events().iter().any(|r| matches!(
				r.event,
				RuntimeEvent::System(frame_system::Event::Remarked { .. })
			)));
		});
	}

	#[test]
	fn xcmp() {
		MockNet::reset();

		let remark = parachain::RuntimeCall::System(
			frame_system::Call::<parachain::Runtime>::remark_with_event { remark: vec![1, 2, 3] },
		);
		ParaA::execute_with(|| {
			assert_ok!(ParachainPalletXcm::send_xcm(
				Here,
				(Parent, Parachain(2)),
				Xcm(vec![Transact {
					origin_kind: OriginKind::SovereignAccount,
					require_weight_at_most: INITIAL_BALANCE as u64,
					call: remark.encode().into(),
				}]),
			));
		});

		ParaB::execute_with(|| {
			use parachain::{RuntimeEvent, System};
			assert!(System::events().iter().any(|r| matches!(
				r.event,
				RuntimeEvent::System(frame_system::Event::Remarked { .. })
			)));
		});
	}

	#[test]
	fn reserve_transfer() {
		MockNet::reset();

		let withdraw_amount = 123;

		Relay::execute_with(|| {
			assert_ok!(RelayChainPalletXcm::reserve_transfer_assets(
				relay_chain::RuntimeOrigin::signed(ALICE),
				Box::new(Parachain(1).into()),
				Box::new(AccountId32 { network: None, id: ALICE.into() }.into()),
				Box::new((Here, withdraw_amount).into()),
				0,
			));
			assert_eq!(
				parachain::Balances::free_balance(&child_account_id(1)),
				INITIAL_BALANCE + withdraw_amount
			);
		});

		ParaA::execute_with(|| {
			// free execution, full amount received
			assert_eq!(
				pallet_balances::Pallet::<parachain::Runtime>::free_balance(&ALICE),
				INITIAL_BALANCE + withdraw_amount
			);
		});
	}

	//////////////////////////////////////////////////////
	///////////////// SCENARIOS START/////////////////////
	//////////////////////////////////////////////////////

	/// Scenario:
	///
	/// Original:
	/// A parachain wants to be notified that a transfer worked correctly.
	/// It sends a `QueryHolding` after the deposit to get notified on success.
	///
	/// Modified:
	/// The example has been modified slightly to demonstrate that correct asset amount is returned.
	/// We withdraw certain amount, but deposit less. We expect that `QueryHolding` will report the remainder.
	///
	/// Asserts that the balances are updated correctly and the expected XCM is sent.
	#[test]
	fn query_holding() {
		MockNet::reset();

		let withdraw_amount = 10;
		let deposit_amount = 3;
		let first_query_id_set = 1234;
		let second_query_id_set = 5678;

		// Send a message which fully succeeds on the relay chain
		ParaA::execute_with(|| {
			let message = Xcm(vec![
				WithdrawAsset((Here, withdraw_amount).into()),
				// We don't deposit everything intentionally, so we can check that `ReportHolding` works as expected
				DepositAsset {
					assets: MultiAsset { id: Concrete(Here.into()), fun: Fungible(deposit_amount) }
						.into(),
					beneficiary: Parachain(2).into(),
				},
				ReportHolding {
					response_info: QueryResponseInfo {
						destination: Parachain(1).into(),
						query_id: first_query_id_set,
						max_weight: 1_000_000_000,
					},
					assets: All.into(), // we choose to report everything, but we could limit it if we wanted to
				},
				ReportHolding {
					response_info: QueryResponseInfo {
						destination: Parachain(1).into(),
						query_id: second_query_id_set,
						max_weight: 1_000_000_000,
					},
					// We repeat almost the same query but we limit it to only native asset and to 1 amount!
					// So even if we have more than 1, at max, 1 can be reported (0 or 1).
					assets: MultiAsset { id: Concrete(Here.into()), fun: Fungible(1) }.into(),
				},
			]);
			// Send withdraw and deposit with query holding
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		// Check that transfer was executed
		Relay::execute_with(|| {
			// Withdraw executed
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(1)),
				INITIAL_BALANCE - withdraw_amount
			);
			// Deposit executed
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(2)),
				INITIAL_BALANCE + deposit_amount
			);
		});

		// Check that QueryResponse message was received
		ParaA::execute_with(|| {
			assert_eq!(
				parachain::MsgQueue::received_dmp(),
				vec![
					Xcm(vec![QueryResponse {
						query_id: first_query_id_set,
						response: Response::Assets(
							(Parent, withdraw_amount - deposit_amount).into()
						),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},]),
					Xcm(vec![QueryResponse {
						query_id: second_query_id_set,
						response: Response::Assets((Parent, 1).into()),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},]) // Notice that we define 2 separate XCM sequences. This is because each `QueryHolding`'s reponse is done individually.
				],
			);

			// TODO: check what happened with the unused funds left in the holding register
		});
	}

	/// Scenario:
	/// A parachain transfers an NFT resident on the relay chain to another parachain account.
	///
	/// Asserts that the parachain accounts are updated as expected.
	#[test]
	fn transfer_asset_nft() {
		MockNet::reset();

		Relay::execute_with(|| {
			assert_eq!(relay_chain::Uniques::owner(1, 42), Some(child_account_id(1)));
		});

		ParaA::execute_with(|| {
			// We want to transfer asset owned by `ParaA` over to `Para2`
			let message = Xcm(vec![TransferAsset {
				assets: (GeneralIndex(1), 42u32).into(),
				beneficiary: Parachain(2).into(),
			}]);
			// We send the message to the `Parent`, the relay chain, since asset is there
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message));
		});

		// Asset ownership has changed
		Relay::execute_with(|| {
			assert_eq!(relay_chain::Uniques::owner(1, 42), Some(child_account_id(2)));
		});
	}

	/// Scenario:
	/// A parachain configures error handler on relay chain, [ReportError]
	/// A parachain attempts to withdraw on relay chain more than it has, causing an error.
	/// We expect to receive error back in a report.
	#[test]
	fn report_error() {
		MockNet::reset();

		let first_query_id = 1234;
		let second_query_id = 5678;
		let max_weight = 1_000_000_000;

		// Send a message which will result in an error on the relay chain
		ParaA::execute_with(|| {
			// First we prepare the sequence for the error handler.
			// The idea is to report actual error, clear error and then report it again.
			// In the first report we expect to see the error but we don't expect it in the second one.
			let error_handler_sequence = Xcm(vec![
				ReportError(QueryResponseInfo {
					destination: Parachain(1).into(),
					query_id: first_query_id,
					max_weight,
				}),
				ClearError,
				ReportError(QueryResponseInfo {
					destination: Parachain(1).into(),
					query_id: second_query_id,
					max_weight,
				}),
			]);

			let message = Xcm(vec![
				// This will set the error handler on the relay chain
				SetErrorHandler(error_handler_sequence),
				// We expect this to fail since it's more than the ParaA account has
				WithdrawAsset((Here, INITIAL_BALANCE + 1).into()),
			]);
			// Send withdraw and deposit with query holding
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		Relay::execute_with(|| {
			// Withdraw was attempted & failed, balance hasn't changed
			assert_eq!(relay_chain::Balances::free_balance(child_account_id(1)), INITIAL_BALANCE);
		});

		// Check that QueryResponse message was received and correct error was reported
		ParaA::execute_with(|| {
			assert_eq!(
				parachain::MsgQueue::received_dmp(),
				vec![
					// We expect the first response to contain an error since we failed to withdraw assets
					Xcm(vec![QueryResponse {
						query_id: first_query_id,
						response: Response::ExecutionResult(Some((
							1, // this is the instruction index at which the error occured
							Error::FailedToTransactAsset("")
						))),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},]),
					// The second response shouldn't contain any errors since we cleared all errors
					Xcm(vec![QueryResponse {
						query_id: second_query_id,
						response: Response::ExecutionResult(None),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},])
				],
			);
		});

		// Error enum can be found here: polkadot/xcm/src/v3/traits.rs
	}

	/// Scenario:
	/// A parachain sets appendix (callback after XCM execution finishes) and executes a remote transaction.
	/// It expects to replies, one with execution error and one with 'success' (should be empty since result was cleared).
	#[test]
	fn set_appendix() {
		MockNet::reset();

		let first_query_id = 1234;
		let second_query_id = 5678;
		let max_weight = 1_000_000_000;

		// Send a message which will set appendix and perform remote transact on the relay chain
		ParaA::execute_with(|| {
			// First we prepare the sequence for the appendix - to be executed after XCM execution finishes (post-call hook).
			// The idea is to report transact status, clear it and then report it again.
			// In the first report we expect to see the execution result (error), while in second we expect success (state after `ClearOrigin`)
			let appendix_sequence = Xcm(vec![
				ReportTransactStatus(QueryResponseInfo {
					destination: Parachain(1).into(),
					query_id: first_query_id,
					max_weight,
				}),
				ClearTransactStatus,
				ReportTransactStatus(QueryResponseInfo {
					destination: Parachain(1).into(),
					query_id: second_query_id,
					max_weight,
				}),
			]);

			// This is what we'll execute on the remote chain, relay in particular.
			// It's a priviliged action and we expect it to fail
			let priviliged_action =
				relay_chain::RuntimeCall::System(
					frame_system::Call::<relay_chain::Runtime>::set_heap_pages { pages: 1337 },
				);

			let message = Xcm(vec![
				// This will set the post process handler on the relay chain
				SetAppendix(appendix_sequence),
				// We expect this to fail since it's more than the ParaA account has
				Transact {
					origin_kind: OriginKind::SovereignAccount,
					require_weight_at_most: 1_000_000_000,
					call: priviliged_action.encode().into(),
				},
			]);

			// Send XCM instructions to the relay chain
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		// process received XCM instructions from the ParaA
		Relay::execute_with(|| {
			// just execute received XCM instructions on the relay chain side
		});

		ParaA::execute_with(|| {
			assert_eq!(
				parachain::MsgQueue::received_dmp(),
				vec![
					// We expect the first response to contain DispatchResult error since we tried to transact root-only priviliged action
					Xcm(vec![QueryResponse {
						query_id: first_query_id,
						response: Response::DispatchResult(MaybeErrorCode::Error(vec![2])),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},]),
					// The second response should be ok since we cleared the transact status prior to sending the response
					Xcm(vec![QueryResponse {
						query_id: second_query_id,
						response: Response::DispatchResult(MaybeErrorCode::Success),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},])
				],
			);
		});
	}

	/// Scenario:
	/// ParaA sets topic on relay chain.
	///
	/// At the moment, I'm not sure how to verify it works in a nice & convenient way. One possible approach would be to
	/// add custom handlers for certain actions which would make use of the topic.
	#[test]
	fn set_topic() {
		MockNet::reset();

		ParaA::execute_with(|| {
			// we want to set a custom topic, which must be [u8; 32]
			let mut topic = vec![0x1u8, 0x3, 0x3, 0x7];
			topic.extend(vec![0u8; 28]);

			let message = Xcm(vec![SetTopic(topic.try_into().unwrap())]);
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message));
		});

		Relay::execute_with(|| {
			// VM is created when XCM execution starts, and is dropped afterwards so I cannot check internal state.
			// TODO: add custom handler so e.g. asset withdraw will work differently based on context?
		});
	}

	/// Scenario:
	/// We get some assets trapped and then claim them using `ClaimAsset` instruction.
	/// We verify the balance state changes on the destination chain, right after they were trapped
	/// and after they were claimed.
	#[test]
	fn claim_asset() {
		MockNet::reset();

		let trap_value = 61;

		ParaA::execute_with(|| {
			// Just withdraw assets and don't do anything with them. Post-processing will trap them.
			let message = Xcm(vec![WithdrawAsset((Here, trap_value).into())]);
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message));
		});

		Relay::execute_with(|| {
			// some assets were withdrawn - and lost! Or trapped, to be more precise.
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(1)),
				INITIAL_BALANCE - trap_value
			);
		});

		ParaA::execute_with(|| {
			// Now we can attempt to claim the trapped assets.
			// It's important we specify EXACTLY the amount of assets that were trapped and use the same origin we used when assets got trapped.
			let message = Xcm(vec![
				ClaimAsset {
					assets: (Here, trap_value).into(),
					ticket: Here.into(), // not important, this value is equivalent to `Don't care`
				},
				DepositAsset { assets: AllCounted(1).into(), beneficiary: Parachain(1).into() },
			]);
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message));
		});

		Relay::execute_with(|| {
			// We can verify that previous balance was restored - trapped assets have been re-claimed.
			assert_eq!(relay_chain::Balances::free_balance(child_account_id(1)), INITIAL_BALANCE);
		});
	}

	/// Scenario:
	/// We register appendix and error handler on the relay chain.
	/// In one case, assets will get deposited to ParaA account, in other to `ParaB` account.
	/// We rely on `ExpectAsset` instruction to raise an error in case expected condition isn't met.
	/// Both scenarios are tested, with and without the error being raised.
	#[test]
	fn expect_asset_branching() {
		MockNet::reset();

		// In case execution results in an error, deposit asset to `ParaA` account
		let error_handler_seq = Xcm(vec![DepositAsset {
			assets: AllCounted(1).into(),
			beneficiary: Parachain(1).into(),
		}]);

		// In case all goes fine, deposit asset to `ParaB` account
		let appendix_handler_seq = Xcm(vec![DepositAsset {
			assets: AllCounted(1).into(),
			beneficiary: Parachain(2).into(),
		}]);

		let send_amount = 29;

		// We will withdraw some assets but will expect to have more in the holding register than we actually have.
		// This is intentional to produce an error.
		ParaA::execute_with(|| {
			let message = Xcm(vec![
				SetErrorHandler(error_handler_seq.clone()),
				SetAppendix(appendix_handler_seq.clone()),
				WithdrawAsset((Here, send_amount).into()),
				ExpectAsset((Here, send_amount + 1).into()),
			]);
			// Send XCM instructions to the relay chain
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		// process received XCM instructions from the ParaA.
		// We expect that error was raised since we expected too much assets in the holding register.
		// In that scenario, all assets that were withdrawn should hav been deposited back to `ParaA`
		Relay::execute_with(|| {
			assert_eq!(relay_chain::Balances::free_balance(child_account_id(1)), INITIAL_BALANCE);

			assert_eq!(relay_chain::Balances::free_balance(child_account_id(2)), INITIAL_BALANCE);
		});

		// We repeat the same sequence but this time we'll expect a bit less than what we have.
		// This shoudl pass and no error should be raised.
		ParaA::execute_with(|| {
			let message = Xcm(vec![
				SetErrorHandler(error_handler_seq.clone()),
				SetAppendix(appendix_handler_seq.clone()),
				WithdrawAsset((Here, send_amount).into()),
				ExpectAsset((Here, send_amount - 1).into()),
			]);
			// Send XCM instructions to the relay chain
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		// process received XCM instructions from the `ParaA`.
		// We expect that assets were deposited to `ParaB` since appendix handler should have been executed.
		Relay::execute_with(|| {
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(1)),
				INITIAL_BALANCE - send_amount
			);

			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(2)),
				INITIAL_BALANCE + send_amount
			);
		});
	}

	/// Scenario
	/// Query system pallet on the relay chain and ensure it's present with expected version.
	#[test]
	fn query_pallet() {
		MockNet::reset();

		let query_id = 1234;
		let max_weight = 1_000_000_000;
		let module_name = "frame_system";

		// Query the relay chain for the system pallet by it's module name
		ParaA::execute_with(|| {
			let message = Xcm(vec![QueryPallet {
				module_name: module_name.into(),
				response_info: QueryResponseInfo {
					destination: Parachain(1).into(),
					query_id,
					max_weight,
				},
			}]);
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone(),));
		});

		Relay::execute_with(|| {
			// execute message received from ParaA, send response back to `ParaA`
		});

		// Check that QueryResponse message was received and pallet info is as expected
		ParaA::execute_with(|| {
			assert_eq!(
				parachain::MsgQueue::received_dmp(),
				vec![
					// We expect the first response to contain an error since we failed to withdraw assets
					Xcm(vec![QueryResponse {
						query_id,
						response: Response::PalletsInfo(
							vec![PalletInfo::new(
								0,                  // index
								"System".into(),    // name
								module_name.into(), // module_name
								4,                  // major
								0,                  // minor
								0,                  // patch
							)
							.unwrap()]
							.try_into()
							.unwrap()
						),
						max_weight: 1_000_000_000,
						querier: Some(Here.into()),
					},])
				],
			);
		});
	}

	// TODO: Idea - maybe demonstrate recursive error handling or appendix setting?

	//////////////////////////////////////////////////////
	///////////////// SCENARIOS END //////////////////////
	//////////////////////////////////////////////////////

	#[test]
	fn remote_locking() {
		MockNet::reset();

		let locked_amount = 100;

		ParaB::execute_with(|| {
			let message = Xcm(vec![LockAsset {
				asset: (Here, locked_amount).into(),
				unlocker: (Parachain(1),).into(),
			}]);
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone()));
		});

		Relay::execute_with(|| {
			use pallet_balances::{BalanceLock, Reasons};
			assert_eq!(
				relay_chain::Balances::locks(&child_account_id(2)),
				vec![BalanceLock {
					id: *b"py/xcmlk",
					amount: locked_amount,
					reasons: Reasons::All
				}]
			);
		});

		ParaA::execute_with(|| {
			assert_eq!(
				parachain::MsgQueue::received_dmp(),
				vec![Xcm(vec![NoteUnlockable {
					owner: (Parent, Parachain(2)).into(),
					asset: (Parent, locked_amount).into()
				}])]
			);
		});
	}

	/// Scenario:
	/// The relay-chain teleports an NFT to a parachain.
	///
	/// Asserts that the parachain accounts are updated as expected.
	#[test]
	fn teleport_nft() {
		MockNet::reset();

		Relay::execute_with(|| {
			// Mint the NFT (1, 69) and give it to our "parachain#1 alias".
			assert_ok!(relay_chain::Uniques::mint(
				relay_chain::RuntimeOrigin::signed(ALICE),
				1,
				69,
				child_account_account_id(1, ALICE),
			));
			// The parachain#1 alias of Alice is what must hold it on the Relay-chain for it to be
			// withdrawable by Alice on the parachain.
			assert_eq!(
				relay_chain::Uniques::owner(1, 69),
				Some(child_account_account_id(1, ALICE))
			);
		});
		ParaA::execute_with(|| {
			assert_ok!(parachain::ForeignUniques::force_create(
				parachain::RuntimeOrigin::root(),
				(Parent, GeneralIndex(1)).into(),
				ALICE,
				false,
			));
			assert_eq!(
				parachain::ForeignUniques::owner((Parent, GeneralIndex(1)).into(), 69u32.into()),
				None,
			);
			assert_eq!(parachain::Balances::reserved_balance(&ALICE), 0);

			// IRL Alice would probably just execute this locally on the Relay-chain, but we can't
			// easily do that here since we only send between chains.
			let message = Xcm(vec![
				WithdrawAsset((GeneralIndex(1), 69u32).into()),
				InitiateTeleport {
					assets: AllCounted(1).into(),
					dest: Parachain(1).into(),
					xcm: Xcm(vec![DepositAsset {
						assets: AllCounted(1).into(),
						beneficiary: (AccountId32 { id: ALICE.into(), network: None },).into(),
					}]),
				},
			]);
			// Send teleport
			let alice = AccountId32 { id: ALICE.into(), network: None };
			assert_ok!(ParachainPalletXcm::send_xcm(alice, Parent, message));
		});
		ParaA::execute_with(|| {
			assert_eq!(
				parachain::ForeignUniques::owner((Parent, GeneralIndex(1)).into(), 69u32.into()),
				Some(ALICE),
			);
			assert_eq!(parachain::Balances::reserved_balance(&ALICE), 1000);
		});
		Relay::execute_with(|| {
			assert_eq!(relay_chain::Uniques::owner(1, 69), None);
		});
	}

	/// Scenario:
	/// The relay-chain transfers an NFT into a parachain's sovereign account, who then mints a
	/// trustless-backed-derivated locally.
	///
	/// Asserts that the parachain accounts are updated as expected.
	#[test]
	fn reserve_asset_transfer_nft() {
		sp_tracing::init_for_tests();
		MockNet::reset();

		Relay::execute_with(|| {
			assert_ok!(relay_chain::Uniques::force_create(
				relay_chain::RuntimeOrigin::root(),
				2,
				ALICE,
				false
			));
			assert_ok!(relay_chain::Uniques::mint(
				relay_chain::RuntimeOrigin::signed(ALICE),
				2,
				69,
				child_account_account_id(1, ALICE)
			));
			assert_eq!(
				relay_chain::Uniques::owner(2, 69),
				Some(child_account_account_id(1, ALICE))
			);
		});
		ParaA::execute_with(|| {
			assert_ok!(parachain::ForeignUniques::force_create(
				parachain::RuntimeOrigin::root(),
				(Parent, GeneralIndex(2)).into(),
				ALICE,
				false,
			));
			assert_eq!(
				parachain::ForeignUniques::owner((Parent, GeneralIndex(2)).into(), 69u32.into()),
				None,
			);
			assert_eq!(parachain::Balances::reserved_balance(&ALICE), 0);

			let message = Xcm(vec![
				WithdrawAsset((GeneralIndex(2), 69u32).into()),
				DepositReserveAsset {
					assets: AllCounted(1).into(),
					dest: Parachain(1).into(),
					xcm: Xcm(vec![DepositAsset {
						assets: AllCounted(1).into(),
						beneficiary: (AccountId32 { id: ALICE.into(), network: None },).into(),
					}]),
				},
			]);
			// Send transfer
			let alice = AccountId32 { id: ALICE.into(), network: None };
			assert_ok!(ParachainPalletXcm::send_xcm(alice, Parent, message));
		});
		ParaA::execute_with(|| {
			log::debug!(target: "xcm-exceutor", "Hello");
			assert_eq!(
				parachain::ForeignUniques::owner((Parent, GeneralIndex(2)).into(), 69u32.into()),
				Some(ALICE),
			);
			assert_eq!(parachain::Balances::reserved_balance(&ALICE), 1000);
		});

		Relay::execute_with(|| {
			assert_eq!(relay_chain::Uniques::owner(2, 69), Some(child_account_id(1)));
		});
	}

	/// Scenario:
	/// The relay-chain creates an asset class on a parachain and then Alice transfers her NFT into
	/// that parachain's sovereign account, who then mints a trustless-backed-derivative locally.
	///
	/// Asserts that the parachain accounts are updated as expected.
	#[test]
	fn reserve_asset_class_create_and_reserve_transfer() {
		MockNet::reset();

		Relay::execute_with(|| {
			assert_ok!(relay_chain::Uniques::force_create(
				relay_chain::RuntimeOrigin::root(),
				2,
				ALICE,
				false
			));
			assert_ok!(relay_chain::Uniques::mint(
				relay_chain::RuntimeOrigin::signed(ALICE),
				2,
				69,
				child_account_account_id(1, ALICE)
			));
			assert_eq!(
				relay_chain::Uniques::owner(2, 69),
				Some(child_account_account_id(1, ALICE))
			);

			let message = Xcm(vec![Transact {
				origin_kind: OriginKind::Xcm,
				require_weight_at_most: 1_000_000_000,
				call: parachain::RuntimeCall::from(
					pallet_uniques::Call::<parachain::Runtime>::create {
						collection: (Parent, 2u64).into(),
						admin: parent_account_id(),
					},
				)
				.encode()
				.into(),
			}]);
			// Send creation.
			assert_ok!(RelayChainPalletXcm::send_xcm(Here, Parachain(1), message));
		});
		ParaA::execute_with(|| {
			// Then transfer
			let message = Xcm(vec![
				WithdrawAsset((GeneralIndex(2), 69u32).into()),
				DepositReserveAsset {
					assets: AllCounted(1).into(),
					dest: Parachain(1).into(),
					xcm: Xcm(vec![DepositAsset {
						assets: AllCounted(1).into(),
						beneficiary: (AccountId32 { id: ALICE.into(), network: None },).into(),
					}]),
				},
			]);
			let alice = AccountId32 { id: ALICE.into(), network: None };
			assert_ok!(ParachainPalletXcm::send_xcm(alice, Parent, message));
		});
		ParaA::execute_with(|| {
			assert_eq!(parachain::Balances::reserved_balance(&parent_account_id()), 1000);
			assert_eq!(
				parachain::ForeignUniques::collection_owner((Parent, 2u64).into()),
				Some(parent_account_id())
			);
		});
	}

	/// Scenario:
	/// A parachain transfers funds on the relay chain to another parachain account.
	///
	/// Asserts that the parachain accounts are updated as expected.
	#[test]
	fn withdraw_and_deposit() {
		MockNet::reset();

		let send_amount = 10;

		ParaA::execute_with(|| {
			let message = Xcm(vec![
				WithdrawAsset((Here, send_amount).into()),
				buy_execution((Here, send_amount)),
				DepositAsset { assets: AllCounted(1).into(), beneficiary: Parachain(2).into() },
			]);
			// Send withdraw and deposit
			assert_ok!(ParachainPalletXcm::send_xcm(Here, Parent, message.clone()));
		});

		Relay::execute_with(|| {
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(1)),
				INITIAL_BALANCE - send_amount
			);
			assert_eq!(
				relay_chain::Balances::free_balance(child_account_id(2)),
				INITIAL_BALANCE + send_amount
			);
		});
	}
}
