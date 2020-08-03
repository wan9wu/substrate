// This file is part of Substrate.

// Copyright (C) 2020 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Benchmarks for the contracts pallet

#![cfg(feature = "runtime-benchmarks")]

use crate::*;
use crate::Module as Contracts;

use frame_benchmarking::{benchmarks, account};
use frame_system::{Module as System, RawOrigin};
use parity_wasm::elements::FuncBody;
use sp_runtime::traits::Hash;

struct Module<T:Trait> {
    code: Vec<u8>,
    hash: <T::Hashing as Hash>::Output,
}

struct Instance<T: Trait> {
    caller: T::AccountId,
    addr: <T::Lookup as StaticLookup>::Source,
    endowment: BalanceOf<T>,
}

macro_rules! load_module {
    ($name:expr) => {{
        let code = include_bytes!(concat!("../fixtures/benchmarks/", $name, ".wat"));
        compile_module::<T>(code)
    }};
}

fn compile_module<T: Trait>(code: &[u8]) -> Result<Module<T>, &str> {
    let text = sp_std::str::from_utf8(code).map_err(|_| "Invalid utf8 in wat file.")?;
    let code = wat::parse_str(text).map_err(|_| "Failed to compile wat file.")?;
    let hash = T::Hashing::hash(&code);
    Ok(Module {
        code,
        hash,
    })
}

fn instantiate_raw_contract<T: Trait>(
    caller_name: &'static str,
    module: Module<T>,
    data: Vec<u8>,
) -> Result<Instance<T>, &'static str>
{
    let endowment = Config::<T>::subsistence_threshold_uncached();
    let caller = create_funded_user::<T>(caller_name, 0);
    let addr = T::DetermineContractAddress::contract_address_for(&module.hash, &[], &caller);
    Contracts::<T>::put_code_raw(module.code)?;
    Contracts::<T>::instantiate(
        RawOrigin::Signed(caller.clone()).into(),
        endowment,
        Weight::max_value(),
        module.hash,
        data,
    )?;
    Ok(Instance {
        caller,
        addr: T::Lookup::unlookup(addr),
        endowment,
    })
}

fn funding<T: Trait>() -> BalanceOf<T> {
    T::Currency::minimum_balance() * 10_000.into()
}

fn create_funded_user<T: Trait>(string: &'static str, n: u32) -> T::AccountId {
	let user = account(string, n, 0);
	T::Currency::make_free_balance_be(&user, funding::<T>());
	user
}

fn contract_with_call_body<T: Trait>(body: FuncBody) -> Module<T> {
    use parity_wasm::elements::{
        Instructions, Instruction::End,
    };
    let contract = parity_wasm::builder::ModuleBuilder::new()
        // deploy function (idx 0)
        .function()
            .signature().with_params(vec![]).with_return_type(None).build()
            .body().with_instructions(Instructions::new(vec![End])).build()
            .build()
        // call function (idx 1)
        .function()
            .signature().with_params(vec![]).with_return_type(None).build()
            .with_body(body)
            .build()
        .export().field("deploy").internal().func(0).build()
        .export().field("call").internal().func(1).build()
        .build();
    let code = contract.to_bytes().unwrap();
    let hash = T::Hashing::hash(&code);
    Module {
        code,
        hash
    }
}

fn expanded_contract<T: Trait>(target_bytes: u32) -> Module<T> {
    use parity_wasm::elements::{
        Instruction::{self, If, I32Const, Return, End},
        BlockType, Instructions,
    };
    // Base size of a contract is 47 bytes and each expansion adds 6 bytes.
    // We do one expansion less to account for the code section and function body
    // size fields inside the binary wasm module representation which are leb128 encoded
    // and therefore grow in size when the contract grows. We are not allowed to overshoot
    // because of the maximum code size that is enforced by `put_code`.
    let expansions = (target_bytes.saturating_sub(47) / 6).saturating_sub(1) as usize;
    const EXPANSION: [Instruction; 4] = [
        I32Const(0),
        If(BlockType::NoResult),
        Return,
        End,
    ];
    let instructions = Instructions::new(
        EXPANSION
            .iter()
            .cycle()
            .take(EXPANSION.len() * expansions)
            .cloned()
            .chain(sp_std::iter::once(End))
            .collect()
    );
    contract_with_call_body::<T>(FuncBody::new(Vec::new(), instructions))
}

fn advance_block<T: Trait>(num: <T as frame_system::Trait>::BlockNumber) {
	let now = System::<T>::block_number();
	System::<T>::set_block_number(now + num);
}

benchmarks! {
    _ {
    }

    // This extrinsic is pretty much constant as it is only a simple setter.
    update_schedule {
        let schedule = Schedule {
            version: 1,
            .. Default::default()
        };
    }: _(RawOrigin::Root, schedule)

    // This constructs a contract that is maximal expensive to instrument.
    // It creates a maximum number of metering blocks per byte.
    put_code {
        let n in 0 .. Contracts::<T>::current_schedule().max_code_size;
        let caller = create_funded_user::<T>("caller", 0);
        let module = expanded_contract::<T>(n);
        let origin = RawOrigin::Signed(caller);
    }: _(origin, module.code)

    // Instantiate uses a dummy contract constructor to measure the overhead of the instantiate.
    // The size of the data has no influence on the costs of this extrinsic as long as the contract
    // won't call `ext_input` in its constructor to copy the data to contract memory.
    // The dummy contract used here does not do this. The costs for the data copy is billed as
    // part of `ext_input`.
    instantiate {
        let data = vec![0u8; 128];
        let endowment = Config::<T>::subsistence_threshold_uncached();
        let caller = create_funded_user::<T>("caller", 0);
        let Module { code, hash } = load_module!("dummy")?;
        let origin = RawOrigin::Signed(caller.clone());
        Contracts::<T>::put_code_raw(code)?;
    }: _(
            origin,
            endowment,
            Weight::max_value(),
            hash,
            data
        )
    verify {
        assert_eq!(
            funding::<T>() - endowment,
            T::Currency::free_balance(&caller),
        )
    }

    // We just call a dummy contract to measure to overhead of the call extrinsic.
    // As for instantiate the size of the data does not influence the costs.
    call {
        let instance = instantiate_raw_contract("caller", load_module!("dummy")?, vec![])?;
        let data = vec![0u8; 128];
        let value = T::Currency::minimum_balance() * 100.into();
        let origin = RawOrigin::Signed(instance.caller.clone());
    }: _(
            origin,
            instance.addr,
            value,
            Weight::max_value(),
            data
        )
    verify {
        assert_eq!(
            funding::<T>() - instance.endowment - value,
            T::Currency::free_balance(&instance.caller),
        )
    }

    // We benchmark the costs for sucessfully evicting an empty contract.
    // The actual costs are depending on how many storage items the evicted contract
    // does have. However, those costs are not to be payed by the sender but
    // will be distributed over multiple blocks using a scheduler. Otherwise there is
    // no incentive to remove large contracts when the removal is more expensive than
    // the reward for removing them.
    claim_surcharge {
        let instance = instantiate_raw_contract("caller", load_module!("dummy")?, vec![])?;
        let origin = RawOrigin::Signed(instance.caller.clone());
        let addr = T::Lookup::lookup(instance.addr.clone())?;
        let addr_clone = addr.clone();

        // instantiate should leave us with an alive contract
        ContractInfoOf::<T>::get(addr.clone()).and_then(|c| c.get_alive())
            .ok_or("Instantiate must return an alive contract.")?;

        // generate some rent
        advance_block::<T>(<T as Trait>::SignedClaimHandicap::get() + 1.into());

    }: _(origin, addr_clone, None)
    verify {
        // the claim surcharge should have evicted the contract
        ContractInfoOf::<T>::get(addr.clone()).and_then(|c| c.get_tombstone())
            .ok_or("The claim surcharge should have exicted the contract.")?;

        // the caller should get the reward for being a good snitch
        assert_eq!(
            funding::<T>() - instance.endowment + <T as Trait>::SurchargeReward::get(),
            T::Currency::free_balance(&instance.caller),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ExtBuilder, Test};
    use frame_support::assert_ok;

    #[test]
    fn update_schedule() {
		ExtBuilder::default().build().execute_with(|| {
			assert_ok!(test_benchmark_update_schedule::<Test>());
		});
    }

    #[test]
    fn put_code() {
		ExtBuilder::default().build().execute_with(|| {
			assert_ok!(test_benchmark_put_code::<Test>());
		});
    }

    #[test]
    fn instantiate() {
		ExtBuilder::default().build().execute_with(|| {
			assert_ok!(test_benchmark_instantiate::<Test>());
		});
    }

    #[test]
    fn call() {
		ExtBuilder::default().build().execute_with(|| {
			assert_ok!(test_benchmark_call::<Test>());
		});
    }

    #[test]
    fn claim_surcharge() {
		ExtBuilder::default().build().execute_with(|| {
			assert_ok!(test_benchmark_claim_surcharge::<Test>());
		});
    }
}
