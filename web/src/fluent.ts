// Registers only the Fluent UI Web Components the app uses and types their tag names.
import {
  Badge,
  BadgeDefinition,
  Button,
  ButtonDefinition,
  Checkbox,
  CheckboxDefinition,
  Dialog,
  DialogBody,
  DialogBodyDefinition,
  DialogDefinition,
  Divider,
  DividerDefinition,
  Drawer,
  DrawerBody,
  DrawerBodyDefinition,
  DrawerDefinition,
  Dropdown,
  DropdownDefinition,
  DropdownOption,
  DropdownOptionDefinition,
  Field,
  FieldDefinition,
  Listbox,
  ListboxDefinition,
  Menu,
  MenuDefinition,
  MenuItem,
  MenuItemDefinition,
  MenuList,
  MenuListDefinition,
  MessageBar,
  MessageBarDefinition,
  Slider,
  SliderDefinition,
  Switch,
  SwitchDefinition,
  TextArea,
  TextAreaDefinition,
  TextInput,
  TextInputDefinition,
} from "@fluentui/web-components";

declare global {
  interface HTMLElementTagNameMap {
    "fluent-badge": Badge;
    "fluent-button": Button;
    "fluent-checkbox": Checkbox;
    "fluent-dialog": Dialog;
    "fluent-dialog-body": DialogBody;
    "fluent-divider": Divider;
    "fluent-drawer": Drawer;
    "fluent-drawer-body": DrawerBody;
    "fluent-dropdown": Dropdown;
    "fluent-option": DropdownOption;
    "fluent-field": Field;
    "fluent-listbox": Listbox;
    "fluent-menu": Menu;
    "fluent-menu-item": MenuItem;
    "fluent-menu-list": MenuList;
    "fluent-message-bar": MessageBar;
    "fluent-slider": Slider;
    "fluent-switch": Switch;
    "fluent-textarea": TextArea;
    "fluent-text-input": TextInput;
  }
}

/** Defines every component; resolves once all custom elements are registered. */
export async function registerFluent(): Promise<void> {
  await Promise.all([
    Badge.define(BadgeDefinition),
    Button.define(ButtonDefinition),
    Checkbox.define(CheckboxDefinition),
    Dialog.define(DialogDefinition),
    DialogBody.define(DialogBodyDefinition),
    Divider.define(DividerDefinition),
    Drawer.define(DrawerDefinition),
    DrawerBody.define(DrawerBodyDefinition),
    Dropdown.define(DropdownDefinition),
    DropdownOption.define(DropdownOptionDefinition),
    Field.define(FieldDefinition),
    Listbox.define(ListboxDefinition),
    Menu.define(MenuDefinition),
    MenuItem.define(MenuItemDefinition),
    MenuList.define(MenuListDefinition),
    MessageBar.define(MessageBarDefinition),
    Slider.define(SliderDefinition),
    Switch.define(SwitchDefinition),
    TextArea.define(TextAreaDefinition),
    TextInput.define(TextInputDefinition),
  ]);
}
