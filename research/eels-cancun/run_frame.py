
import argparse

from ethereum_types.bytes import Bytes32
from ethereum_types.numeric import U64, U256, Uint

from ethereum.state import Address
from ethereum.state_mpt import State, state_root

from ethereum.forks.cancun.state_tracker import (
    BlockState,
    TransactionState,
)
from ethereum.forks.cancun.vm import (
    BlockEnvironment,
    TransactionEnvironment,
    Message,
    Evm,
)
from ethereum.forks.cancun.vm.interpreter import process_message


def run_frame(
    bytecode: bytes,
    frame_gas: int,
    tx_gas: int | None = None,
) -> Evm:
    """Execute an isolated EVM frame using Cancun EELS."""

    if tx_gas is None:
        tx_gas = frame_gas

    # 1. Empty pre-execution state
    pre_state = State()
    block_state = BlockState(pre_state=pre_state)
    tx_state = TransactionState(parent=block_state)

    # 2. Synthetic addresses
    caller = Address(bytes.fromhex("11" * 20))
    target = Address(bytes.fromhex("22" * 20))
    coinbase = Address(bytes(20))

    # 3. Block environment
    block_env = BlockEnvironment(
        chain_id=U64(1),
        state=block_state,
        block_gas_limit=Uint(30_000_000),
        block_hashes=[],
        coinbase=coinbase,
        number=Uint(1),
        base_fee_per_gas=Uint(0),
        time=U256(0),
        prev_randao=Bytes32(bytes(32)),
        excess_blob_gas=U64(0),
        parent_beacon_block_root=Bytes32(bytes(32)),
    )

    # 4. Transaction environment
    tx_env = TransactionEnvironment(
        origin=caller,
        gas_price=Uint(0),
        gas=Uint(tx_gas),
        access_list_addresses=set(),
        access_list_storage_keys=set(),
        state=tx_state,
        blob_versioned_hashes=(),
        index_in_block=None,
        tx_hash=None,
    )

    # 5. Root execution frame
    message = Message(
        block_env=block_env,
        tx_env=tx_env,
        caller=caller,
        target=target,
        current_target=target,
        gas=Uint(frame_gas),
        value=U256(0),
        data=b"",
        code_address=target,
        code=bytecode,
        depth=Uint(0),
        should_transfer_value=False,
        is_static=False,
        accessed_addresses=set(),
        accessed_storage_keys=set(),
        parent_evm=None,
    )

    # 6. Execute directly, bypassing transaction processing
    return process_message(message)


def main():
    parser = argparse.ArgumentParser(
        description="Execute an isolated Cancun EVM frame using EELS."
    )

    parser.add_argument(
        "--code",
        required=True,
        help="EVM bytecode in hexadecimal",
    )

    parser.add_argument(
        "--gas",
        type=int,
        required=True,
        help="Initial execution frame gas",
    )

    parser.add_argument(
        "--tx-gas",
        type=int,
        default=None,
        help="Transaction environment gas (defaults to frame gas)",
    )

    args = parser.parse_args()

    try:
        bytecode = bytes.fromhex(args.code.removeprefix("0x"))
    except ValueError:
        parser.error("Invalid hexadecimal bytecode")

    if args.gas < 0:
        parser.error("Frame gas must be non-negative")

    if args.tx_gas is not None and args.tx_gas < 0:
        parser.error("Transaction gas must be non-negative")

    tx_gas = args.tx_gas if args.tx_gas is not None else args.gas

    evm = run_frame(
        bytecode=bytecode,
        frame_gas=args.gas,
        tx_gas=tx_gas,
    )

    # 7. Execution observations
    print("Bytecode:", bytecode.hex())
    print("Initial txEnv gas:", tx_gas)
    print("Initial Msg gas:", args.gas)
    print("Remaining gas:", int(evm.gas_left))
    print("Gas consumed:", args.gas - int(evm.gas_left))
    print("Stack:", [int(value) for value in evm.stack])
    print("PC:", int(evm.pc))
    print("Success:", evm.error is None)
    print("Error:", repr(evm.error))
    print(
        "Error type:",
        type(evm.error).__name__ if evm.error is not None else None,
    )
    print(
        "Pre-state root:",
        state_root(evm.message.block_env.state.pre_state).hex(),
    )


if __name__ == "__main__":
    main()
